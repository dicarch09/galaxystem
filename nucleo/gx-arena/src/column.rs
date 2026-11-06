// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fixed-address columnar storage backed by anonymous virtual memory
//! (F0-diseno.md §1.8).
//!
//! A [`Column`] reserves virtual memory for its whole capacity up front with
//! `mmap(MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE)`. Elements are reserved
//! in blocks with an atomic length, so addresses never move and no
//! reallocation ever happens.
//!
//! # Concurrency model (F0.2)
//!
//! While writable, every element is stored in an atomic cell
//! ([`Element::Cell`]) and accessed with `Relaxed` loads and stores, so a
//! `Column` is `Sync` and concurrent access can never be a data race. On
//! x86_64 and aarch64 these are plain loads and stores. Visibility between
//! threads is the caller's responsibility (the scanner publishes nodes
//! through the work queues, which synchronise).
//!
//! [`Column::freeze`] consumes the column — so no other access can exist —
//! and reinterprets the cells as plain `T` ([`Frozen`]); each cell type has
//! the same size, alignment and bit validity as its `T`.

use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::ops::{Deref, Range};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering};

use rustix::io::Errno;
use rustix::mm::{MapFlags, ProtFlags, mmap_anonymous, munmap};

/// Plain data that can be stored in a [`Column`].
///
/// # Safety
///
/// - `Self` is `Copy`, non-zero-sized and owns no resources;
/// - the all-zero bit pattern is a valid `Self` and a valid `Cell`
///   (reserved elements are read from zero-filled anonymous memory);
/// - `Cell` has the same size, alignment and bit validity as `Self`, so a
///   frozen column of cells can be read as a slice of `Self`.
pub unsafe trait Element: Copy + 'static {
    /// Atomic storage cell used while the column is writable.
    type Cell: Sync + 'static;
    /// Loads the value of a cell.
    fn load(cell: &Self::Cell) -> Self;
    /// Stores a value into a cell.
    fn store(cell: &Self::Cell, value: Self);
}

macro_rules! atomic_element {
    ($t:ty, $a:ty) => {
        // SAFETY: the standard library guarantees that `$a` has the same size,
        // alignment (on the 64-bit targets we build for; checked below) and
        // bit validity as `$t`; zero is valid for both.
        unsafe impl Element for $t {
            type Cell = $a;
            #[inline]
            fn load(cell: &$a) -> $t {
                cell.load(Ordering::Relaxed)
            }
            #[inline]
            fn store(cell: &$a, value: $t) {
                cell.store(value, Ordering::Relaxed)
            }
        }
        const _: () = assert!(size_of::<$a>() == size_of::<$t>());
        const _: () = assert!(align_of::<$a>() == align_of::<$t>());
    };
}

atomic_element!(u8, AtomicU8);
atomic_element!(u16, AtomicU16);
atomic_element!(u32, AtomicU32);
atomic_element!(u64, AtomicU64);

/// Atomic cell of a [`crate::tables::DirRow`].
#[repr(C)]
pub struct DirRowCell {
    node: AtomicU32,
    first_child: AtomicU32,
    child_count: AtomicU32,
}

// SAFETY: `DirRow` and `DirRowCell` are both `#[repr(C)]` with three 4-byte
// fields (`u32` vs `AtomicU32`, same size, alignment and bit validity), so
// they have identical layout; zero is valid for both. A row is three
// independent atomic fields, not one atomic value: each field is written by
// a single thread (see the scanner), so readers never mix two writes.
unsafe impl Element for crate::tables::DirRow {
    type Cell = DirRowCell;
    fn load(c: &DirRowCell) -> Self {
        crate::tables::DirRow {
            node: c.node.load(Ordering::Relaxed),
            first_child: c.first_child.load(Ordering::Relaxed),
            child_count: c.child_count.load(Ordering::Relaxed),
        }
    }
    fn store(c: &DirRowCell, v: Self) {
        c.node.store(v.node, Ordering::Relaxed);
        c.first_child.store(v.first_child, Ordering::Relaxed);
        c.child_count.store(v.child_count, Ordering::Relaxed);
    }
}
const _: () = assert!(size_of::<DirRowCell>() == size_of::<crate::tables::DirRow>());
const _: () = assert!(align_of::<DirRowCell>() == align_of::<crate::tables::DirRow>());

/// Errors returned by [`Column`] operations. None of them leave the column in
/// an inconsistent state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ColumnError {
    /// The reservation does not fit in the remaining capacity.
    #[error(
        "column capacity exhausted: {requested} more elements requested, {len} of {capacity} used"
    )]
    CapacityExhausted {
        /// Elements requested.
        requested: u32,
        /// Elements already reserved.
        len: u32,
        /// Maximum number of elements.
        capacity: u32,
    },
    /// The index is not below the reserved length.
    #[error("index {index} out of range (len {len})")]
    OutOfRange {
        /// Index that was accessed.
        index: u32,
        /// Reserved length at the time of the access.
        len: u32,
    },
    /// The kernel refused to reserve the virtual memory.
    #[error("cannot reserve {bytes} bytes of virtual memory: {errno}")]
    Map {
        /// Size of the requested mapping.
        bytes: usize,
        /// Error returned by `mmap`.
        errno: Errno,
    },
}

impl ColumnError {
    /// Whether the error is a virtual memory reservation failure that a
    /// smaller capacity might avoid.
    pub fn is_out_of_memory(&self) -> bool {
        matches!(self, ColumnError::Map { errno, .. } if *errno == Errno::NOMEM)
    }
}

/// Owned anonymous mapping. Unmapped on drop.
struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: `Mapping` exclusively owns a range of anonymous memory; moving or
// sharing the owner across threads is like moving or sharing a heap buffer.
// What may be done with the memory is controlled by `Column` and `Frozen`.
unsafe impl Send for Mapping {}
// SAFETY: as above.
unsafe impl Sync for Mapping {}

impl Mapping {
    fn new(bytes: usize) -> Result<Self, ColumnError> {
        if bytes == 0 {
            return Ok(Mapping {
                ptr: NonNull::dangling(),
                len: 0,
            });
        }
        let page = rustix::param::page_size();
        let len = bytes
            .checked_next_multiple_of(page)
            .ok_or(ColumnError::Map {
                bytes,
                errno: Errno::NOMEM,
            })?;
        // SAFETY: a null address hint lets the kernel pick a fresh range, so
        // the new mapping cannot alias any existing Rust allocation. No
        // `MAP_FIXED` is used.
        let raw = unsafe {
            mmap_anonymous(
                core::ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::PRIVATE | MapFlags::NORESERVE,
            )
        }
        .map_err(|errno| ColumnError::Map { bytes: len, errno })?;
        let ptr = NonNull::new(raw.cast::<u8>()).ok_or(ColumnError::Map {
            bytes: len,
            errno: Errno::NOMEM,
        })?;
        Ok(Mapping { ptr, len })
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        if self.len > 0 {
            // SAFETY: `ptr`/`len` describe exactly the mapping created in
            // `Mapping::new`, which this value owns exclusively. Its owners
            // (`Column`, `Frozen`) only lend references tied to their own
            // lifetime, so none can outlive this drop.
            let result = unsafe { munmap(self.ptr.as_ptr().cast(), self.len) };
            debug_assert!(result.is_ok(), "munmap failed: {result:?}");
        }
    }
}

/// Growable column with a fixed maximum capacity and stable addresses.
pub struct Column<T: Element> {
    map: Mapping,
    capacity: u32,
    len: AtomicU32,
    _elem: PhantomData<T>,
}

impl<T: Element> Column<T> {
    /// Reserves virtual memory for `capacity` elements.
    pub fn with_capacity(capacity: u32) -> Result<Self, ColumnError> {
        const { assert!(size_of::<T>() > 0, "zero-sized column elements") };
        let bytes = (capacity as usize)
            .checked_mul(size_of::<T>())
            .ok_or(ColumnError::Map {
                bytes: usize::MAX,
                errno: Errno::NOMEM,
            })?;
        Ok(Column {
            map: Mapping::new(bytes)?,
            capacity,
            len: AtomicU32::new(0),
            _elem: PhantomData,
        })
    }

    /// Number of reserved elements.
    pub fn len(&self) -> u32 {
        self.len.load(Ordering::Acquire)
    }

    /// Whether no element has been reserved.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Maximum number of elements.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Reserves `n` consecutive elements, initially zero, and returns their
    /// index range. Concurrent reservations get disjoint ranges. Fails without
    /// side effects if the capacity is exhausted.
    pub fn reserve(&self, n: u32) -> Result<Range<u32>, ColumnError> {
        let mut current = self.len.load(Ordering::Relaxed);
        loop {
            let end = current
                .checked_add(n)
                .filter(|&end| end <= self.capacity)
                .ok_or(ColumnError::CapacityExhausted {
                    requested: n,
                    len: current,
                    capacity: self.capacity,
                })?;
            match self
                .len
                .compare_exchange_weak(current, end, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return Ok(current..end),
                Err(actual) => current = actual,
            }
        }
    }

    /// Extends the reserved length to at least `end` (no-op if already
    /// longer). Used to keep secondary columns in step with a primary one.
    pub fn ensure_len(&self, end: u32) -> Result<(), ColumnError> {
        if end > self.capacity {
            return Err(ColumnError::CapacityExhausted {
                requested: end.saturating_sub(self.len()),
                len: self.len(),
                capacity: self.capacity,
            });
        }
        self.len.fetch_max(end, Ordering::AcqRel);
        Ok(())
    }

    /// Reserves one element and writes `value` into it.
    pub fn push(&self, value: T) -> Result<u32, ColumnError> {
        let index = self.reserve(1)?.start;
        self.set(index, value)?;
        Ok(index)
    }

    /// The cell at `index`, if reserved.
    pub fn cell(&self, index: u32) -> Option<&T::Cell> {
        if index >= self.len() {
            return None;
        }
        // SAFETY: `index < len <= capacity`, so the cell lies inside the
        // mapping, which is page-aligned (a multiple of any cell's alignment).
        // The memory is zero-filled or written through cells, and zero is a
        // valid cell (`Element` contract). Cells are atomics, so shared
        // references to them may coexist with concurrent access. The
        // reference borrows `self`, which owns the mapping.
        Some(unsafe { &*self.map.ptr.as_ptr().cast::<T::Cell>().add(index as usize) })
    }

    /// Loads the element at `index`, or `None` if it is not reserved.
    #[inline]
    pub fn get(&self, index: u32) -> Option<T> {
        self.cell(index).map(T::load)
    }

    /// Writes `value` at an already reserved `index`.
    #[inline]
    pub fn set(&self, index: u32, value: T) -> Result<(), ColumnError> {
        match self.cell(index) {
            Some(c) => {
                T::store(c, value);
                Ok(())
            }
            None => Err(ColumnError::OutOfRange {
                index,
                len: self.len(),
            }),
        }
    }

    /// Writes `values` into the already reserved range starting at `start`.
    pub fn set_slice(&self, start: u32, values: &[T]) -> Result<(), ColumnError> {
        let len = self.len();
        let end = u32::try_from(values.len())
            .ok()
            .and_then(|n| start.checked_add(n));
        match end {
            Some(end) if end <= len => {}
            _ => return Err(ColumnError::OutOfRange { index: start, len }),
        }
        for (i, &v) in values.iter().enumerate() {
            self.set(start + i as u32, v)?;
        }
        Ok(())
    }

    /// Consumes the column and returns its reserved elements as a read-only,
    /// shareable slice.
    pub fn freeze(self) -> Frozen<T> {
        let len = self.len();
        Frozen {
            map: self.map,
            len,
            _elem: PhantomData,
        }
    }
}

impl Column<u16> {
    /// Atomically ORs `bits` into the element at `index`.
    pub fn fetch_or(&self, index: u32, bits: u16) -> Result<u16, ColumnError> {
        self.cell(index)
            .map(|c| c.fetch_or(bits, Ordering::Relaxed))
            .ok_or(ColumnError::OutOfRange {
                index,
                len: self.len(),
            })
    }
}

impl Column<u32> {
    /// Atomically subtracts `v` with acquire-release ordering and returns the
    /// previous value.
    pub fn fetch_sub(&self, index: u32, v: u32) -> Result<u32, ColumnError> {
        self.cell(index)
            .map(|c| c.fetch_sub(v, Ordering::AcqRel))
            .ok_or(ColumnError::OutOfRange {
                index,
                len: self.len(),
            })
    }
}

impl Column<u64> {
    /// Atomically adds `v` (relaxed) and returns the previous value.
    pub fn fetch_add(&self, index: u32, v: u64) -> Result<u64, ColumnError> {
        self.cell(index)
            .map(|c| c.fetch_add(v, Ordering::Relaxed))
            .ok_or(ColumnError::OutOfRange {
                index,
                len: self.len(),
            })
    }
}

/// Read-only view of a frozen [`Column`]. Dereferences to `[T]`.
pub struct Frozen<T: Element> {
    map: Mapping,
    len: u32,
    _elem: PhantomData<T>,
}

impl<T: Element> Deref for Frozen<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` cells lie inside the owned mapping (the
        // column checked `len <= capacity`) and are aligned. Each cell has the
        // layout and bit validity of `T` (`Element` contract), so they can be
        // read as `T`s. `freeze` consumed the column, and every write to it
        // happened before (the column was owned exclusively when frozen), so
        // nothing writes to this memory any more; the slice borrows `self`,
        // so it cannot outlive the mapping.
        unsafe { core::slice::from_raw_parts(self.map.ptr.as_ptr().cast::<T>(), self.len as usize) }
    }
}

impl<T: Element + core::fmt::Debug> core::fmt::Debug for Frozen<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_write_read() {
        let col = Column::<u64>::with_capacity(1000).unwrap();
        assert!(col.is_empty());
        let a = col.reserve(10).unwrap();
        let b = col.reserve(5).unwrap();
        assert_eq!(a, 0..10);
        assert_eq!(b, 10..15);
        assert_eq!(col.len(), 15);
        // Reserved elements start as zero.
        assert_eq!(col.get(3), Some(0));
        col.set(3, 42).unwrap();
        col.set(14, u64::MAX).unwrap();
        assert_eq!(col.get(3), Some(42));
        assert_eq!(col.get(14), Some(u64::MAX));
        assert_eq!(col.push(7).unwrap(), 15);
        assert_eq!(col.fetch_add(15, 3).unwrap(), 7);
        let frozen = col.freeze();
        assert_eq!(frozen.len(), 16);
        assert_eq!(frozen[3], 42);
        assert_eq!(frozen[15], 10);
    }

    #[test]
    fn out_of_range_access_is_an_error() {
        let col = Column::<u32>::with_capacity(100).unwrap();
        col.reserve(4).unwrap();
        assert_eq!(col.get(4), None);
        assert_eq!(col.get(u32::MAX), None);
        assert_eq!(
            col.set(4, 1),
            Err(ColumnError::OutOfRange { index: 4, len: 4 })
        );
        assert!(col.fetch_sub(4, 1).is_err());
        assert!(col.set_slice(3, &[1, 2]).is_err());
        // Capacity is not the same as length: index 50 is mapped but not reserved.
        assert_eq!(col.get(50), None);
    }

    #[test]
    fn exhausted_capacity_is_an_error_without_side_effects() {
        let col = Column::<u16>::with_capacity(10).unwrap();
        col.reserve(8).unwrap();
        let err = col.reserve(3).unwrap_err();
        assert_eq!(
            err,
            ColumnError::CapacityExhausted {
                requested: 3,
                len: 8,
                capacity: 10
            }
        );
        assert_eq!(col.len(), 8);
        assert_eq!(col.reserve(2).unwrap(), 8..10);
        assert!(col.reserve(1).is_err());
        assert!(col.push(1).is_err());
        assert_eq!(col.reserve(0).unwrap(), 10..10);
        // u32 overflow in the length computation is also an error.
        assert!(col.reserve(u32::MAX).is_err());
    }

    #[test]
    fn ensure_len_extends_only_forward() {
        let col = Column::<u8>::with_capacity(16).unwrap();
        col.ensure_len(5).unwrap();
        assert_eq!(col.len(), 5);
        col.ensure_len(2).unwrap();
        assert_eq!(col.len(), 5);
        assert!(col.ensure_len(17).is_err());
        assert_eq!(col.len(), 5);
    }

    #[test]
    fn zero_capacity_column() {
        let col = Column::<u64>::with_capacity(0).unwrap();
        assert!(col.reserve(1).is_err());
        assert_eq!(col.reserve(0).unwrap(), 0..0);
        let frozen = col.freeze();
        assert!(frozen.is_empty());
    }

    #[test]
    fn large_sparse_reservation_does_not_commit_memory() {
        // 2^28 u64 = 2 GiB of virtual memory; only the touched page is used.
        let col = Column::<u64>::with_capacity(1 << 28).unwrap();
        let r = col.reserve(1 << 28).unwrap();
        col.set(r.end - 1, 9).unwrap();
        assert_eq!(col.get(r.end - 1), Some(9));
    }

    #[test]
    fn struct_rows() {
        use crate::tables::DirRow;
        let col = Column::<DirRow>::with_capacity(4).unwrap();
        let i = col
            .push(DirRow {
                node: 1,
                first_child: 2,
                child_count: 3,
            })
            .unwrap();
        assert_eq!(col.get(i).unwrap().child_count, 3);
        assert_eq!(col.freeze()[0].first_child, 2);
    }

    #[test]
    fn concurrent_reservations_are_disjoint_and_complete() {
        let col = Column::<u32>::with_capacity(1 << 20).unwrap();
        std::thread::scope(|s| {
            for t in 0..8u32 {
                let col = &col;
                s.spawn(move || {
                    for _ in 0..1000 {
                        let r = col.reserve(7).unwrap();
                        for i in r {
                            col.set(i, t + 1).unwrap();
                        }
                    }
                });
            }
        });
        let frozen = col.freeze();
        assert_eq!(frozen.len(), 8 * 1000 * 7);
        assert!(frozen.iter().all(|&v| (1..=8).contains(&v)));
        // Each block of 7 was written by a single thread.
        for block in frozen.chunks(7) {
            assert!(block.iter().all(|&v| v == block[0]));
        }
        for t in 1..=8 {
            assert_eq!(frozen.iter().filter(|&&v| v == t).count(), 7000);
        }
    }
}
