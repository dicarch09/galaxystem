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
//! # Aliasing model (F0.1)
//!
//! A `Column` is `Send` but not `Sync`. While it is writable it never hands
//! out references into its memory: [`Column::get`] copies the element out and
//! [`Column::set`] writes through a raw pointer, so no reference can overlap a
//! write. [`Column::freeze`] consumes the column and only then exposes the
//! memory as a shared slice ([`Frozen`]), which is `Sync`.

use core::marker::PhantomData;
use core::mem::size_of;
use core::ops::{Deref, Range};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, Ordering};

use rustix::io::Errno;
use rustix::mm::{MapFlags, ProtFlags, mmap_anonymous, munmap};

/// Plain data that can be stored in a [`Column`].
///
/// # Safety
///
/// Implementors must be `Copy`, have a non-zero size, own no resources, and
/// the all-zero bit pattern must be a valid value: freshly reserved elements
/// are read from zero-filled anonymous memory.
pub unsafe trait Zeroable: Copy + 'static {}

// SAFETY: primitive integers are Copy, sized, own nothing, and zero is valid.
unsafe impl Zeroable for u8 {}
// SAFETY: as above.
unsafe impl Zeroable for u16 {}
// SAFETY: as above.
unsafe impl Zeroable for u32 {}
// SAFETY: as above.
unsafe impl Zeroable for u64 {}
// SAFETY: `DirRow` is `#[repr(C)]` with three `u32` fields: Copy, no
// resources, every bit pattern (including all zeros) is valid.
unsafe impl Zeroable for crate::tables::DirRow {}

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
pub struct Column<T: Zeroable> {
    map: Mapping,
    capacity: u32,
    len: AtomicU32,
    _elem: PhantomData<T>,
}

// SAFETY: a `Column` exclusively owns its mapping; sending it to another
// thread transfers that ownership. `T: Send` covers the stored values. It is
// deliberately not `Sync` (see the module documentation).
unsafe impl<T: Zeroable + Send> Send for Column<T> {}

impl<T: Zeroable> Column<T> {
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
    /// index range. Fails without side effects if the capacity is exhausted.
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

    /// Copies out the element at `index`, or `None` if it is not reserved.
    pub fn get(&self, index: u32) -> Option<T> {
        if index >= self.len() {
            return None;
        }
        // SAFETY: `index < len <= capacity`, so the element lies inside the
        // mapping, which is aligned to the page size (a multiple of any
        // `T`'s alignment). The bytes are either zero or a previously written
        // `T`, both valid by the `Zeroable` contract. The column is not
        // `Sync` and never lends references while writable, so no write can
        // overlap this read.
        Some(unsafe { self.base().add(index as usize).read() })
    }

    /// Writes `value` at an already reserved `index`.
    pub fn set(&self, index: u32, value: T) -> Result<(), ColumnError> {
        let len = self.len();
        if index >= len {
            return Err(ColumnError::OutOfRange { index, len });
        }
        // SAFETY: in bounds and aligned as in `get`. The mapping is writable
        // and no reference into it exists while the column is writable, so
        // writing through the raw pointer cannot invalidate a live borrow.
        unsafe { self.base().add(index as usize).write(value) };
        Ok(())
    }

    /// Copies `values` into the already reserved range starting at `start`.
    pub fn set_slice(&self, start: u32, values: &[T]) -> Result<(), ColumnError> {
        let len = self.len();
        let end = u32::try_from(values.len())
            .ok()
            .and_then(|n| start.checked_add(n));
        match end {
            Some(end) if end <= len => {}
            _ => return Err(ColumnError::OutOfRange { index: start, len }),
        }
        // SAFETY: `start + values.len() <= len <= capacity`, so the
        // destination range is inside the writable mapping and aligned. The
        // source is a Rust slice, which cannot point into this mapping
        // because the column never lends references while writable, so the
        // ranges do not overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(
                values.as_ptr(),
                self.base().add(start as usize),
                values.len(),
            );
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

    fn base(&self) -> *mut T {
        self.map.ptr.as_ptr().cast::<T>()
    }
}

/// Read-only view of a frozen [`Column`]. Dereferences to `[T]`.
pub struct Frozen<T: Zeroable> {
    map: Mapping,
    len: u32,
    _elem: PhantomData<T>,
}

// SAFETY: `Frozen` owns its mapping and only allows shared, read-only access
// to it; sending or sharing it across threads is like sending or sharing a
// `Box<[T]>` with `T: Send + Sync`.
unsafe impl<T: Zeroable + Send> Send for Frozen<T> {}
// SAFETY: as above; the memory is never written after freezing.
unsafe impl<T: Zeroable + Sync> Sync for Frozen<T> {}

impl<T: Zeroable> Deref for Frozen<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` elements lie inside the owned mapping (the
        // column checked `len <= capacity`), are properly aligned and hold
        // valid `T`s (zero or written). Nothing writes to the mapping after
        // `freeze`, and the slice borrows `self`, so it cannot outlive the
        // mapping.
        unsafe { core::slice::from_raw_parts(self.map.ptr.as_ptr().cast::<T>(), self.len as usize) }
    }
}

impl<T: Zeroable + core::fmt::Debug> core::fmt::Debug for Frozen<T> {
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
        let frozen = col.freeze();
        assert_eq!(frozen.len(), 16);
        assert_eq!(frozen[3], 42);
        assert_eq!(frozen[15], 7);
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
    }
}
