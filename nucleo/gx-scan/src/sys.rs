// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Raw system interfaces not covered by rustix's safe API. The only module
//! of this crate allowed to contain `unsafe` code.

use rustix::fd::AsFd;
use rustix::io;
use rustix::ioctl::{Getter, ioctl, opcode};

/// `struct btrfs_ioctl_timespec` from `<linux/btrfs.h>`.
#[repr(C)]
#[derive(Clone, Copy)]
struct BtrfsTimespec {
    sec: u64,
    nsec: u32,
}

/// `struct btrfs_ioctl_get_subvol_info_args` from `<linux/btrfs.h>`.
#[repr(C)]
#[derive(Clone, Copy)]
struct GetSubvolInfoArgs {
    treeid: u64,
    name: [u8; 256],
    parent_id: u64,
    dirid: u64,
    generation: u64,
    flags: u64,
    uuid: [u8; 16],
    parent_uuid: [u8; 16],
    received_uuid: [u8; 16],
    ctransid: u64,
    otransid: u64,
    stransid: u64,
    rtransid: u64,
    ctime: BtrfsTimespec,
    otime: BtrfsTimespec,
    stime: BtrfsTimespec,
    rtime: BtrfsTimespec,
    reserved: [u64; 8],
}

// The opcode encodes the struct size; a layout mismatch would make the
// kernel write past or short of our buffer.
const _: () = assert!(size_of::<GetSubvolInfoArgs>() == 504);
const _: () = assert!(size_of::<BtrfsTimespec>() == 16);

const BTRFS_IOCTL_MAGIC: u8 = 0x94;

/// `BTRFS_IOC_GET_SUBVOL_INFO = _IOR(BTRFS_IOCTL_MAGIC, 60, struct
/// btrfs_ioctl_get_subvol_info_args)`. Unprivileged since Linux 4.18.
const BTRFS_IOC_GET_SUBVOL_INFO: rustix::ioctl::Opcode =
    opcode::read::<GetSubvolInfoArgs>(BTRFS_IOCTL_MAGIC, 60);

/// Subset of the subvolume information used by the scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubvolInfo {
    /// Subvolume generation.
    pub generation: u64,
    /// UUID of the subvolume this one is a snapshot of (zero if none).
    pub parent_uuid: [u8; 16],
}

/// Queries `BTRFS_IOC_GET_SUBVOL_INFO` on an open directory of a btrfs
/// filesystem.
pub fn btrfs_subvol_info<Fd: AsFd>(fd: Fd) -> io::Result<SubvolInfo> {
    // SAFETY: `BTRFS_IOC_GET_SUBVOL_INFO` is a valid read-only opcode, and
    // for it the kernel writes exactly one `btrfs_ioctl_get_subvol_info_args`,
    // which `GetSubvolInfoArgs` mirrors field by field with `#[repr(C)]` (its
    // size is asserted above). The struct only holds integers, so whatever
    // the kernel writes is a valid value.
    let args = unsafe {
        ioctl(
            fd,
            Getter::<BTRFS_IOC_GET_SUBVOL_INFO, GetSubvolInfoArgs>::new(),
        )
    }?;
    Ok(SubvolInfo {
        generation: args.generation,
        parent_uuid: args.parent_uuid,
    })
}

/// Batched `statx` through io_uring (`IORING_OP_STATX`), for the A/B of
/// F0-diseno.md §2.2. One ring per scanner thread.
#[cfg(feature = "uring")]
pub struct StatxRing {
    ring: io_uring::IoUring,
    /// NUL-terminated names of the batch in flight.
    paths: Vec<u8>,
    offsets: Vec<usize>,
    /// Output buffers of the batch in flight.
    bufs: Vec<std::mem::MaybeUninit<rustix::fs::Statx>>,
    /// Set if a batch could not be drained; the ring must not be used again.
    broken: bool,
}

/// Placeholder when the `uring` feature is disabled: it cannot be created.
#[cfg(not(feature = "uring"))]
pub enum StatxRing {}

#[cfg(not(feature = "uring"))]
impl StatxRing {
    /// Always fails: io_uring support was not compiled in.
    pub fn new(_entries: u32) -> io::Result<Self> {
        Err(io::Errno::NOSYS)
    }

    /// Unreachable: no value of this type exists.
    pub fn statx_all<'n>(
        &mut self,
        _dir: rustix::fd::BorrowedFd<'_>,
        _names: impl Iterator<Item = &'n [u8]>,
        _flags: rustix::fs::AtFlags,
        _mask: rustix::fs::StatxFlags,
        _out: &mut Vec<io::Result<rustix::fs::Statx>>,
    ) {
        match *self {}
    }
}

// The kernel writes a `struct statx` (256 bytes) through the pointer, and
// rustix's `Statx` is that struct.
#[cfg(feature = "uring")]
const _: () = assert!(size_of::<rustix::fs::Statx>() == 256);

#[cfg(feature = "uring")]
impl StatxRing {
    /// Creates a ring with `entries` submission slots. Fails if io_uring is
    /// unavailable (disabled by `kernel.io_uring_disabled`, seccomp, …) or
    /// does not support `IORING_OP_STATX`.
    pub fn new(entries: u32) -> io::Result<Self> {
        let to_errno = |e: std::io::Error| io::Errno::from_io_error(&e).unwrap_or(io::Errno::NOSYS);
        let ring = io_uring::IoUring::new(entries).map_err(to_errno)?;
        let mut probe = io_uring::Probe::new();
        ring.submitter()
            .register_probe(&mut probe)
            .map_err(to_errno)?;
        if !probe.is_supported(io_uring::opcode::Statx::CODE) {
            return Err(io::Errno::NOSYS);
        }
        Ok(StatxRing {
            ring,
            paths: Vec::new(),
            offsets: Vec::new(),
            bufs: Vec::new(),
            broken: false,
        })
    }

    /// Runs `statx(dir, name, flags, mask)` for every name and stores the
    /// results in `out`, in the same order. If the ring fails, the rest of
    /// the batch is done with synchronous `statx`.
    pub fn statx_all<'n>(
        &mut self,
        dir: rustix::fd::BorrowedFd<'_>,
        names: impl Iterator<Item = &'n [u8]>,
        flags: rustix::fs::AtFlags,
        mask: rustix::fs::StatxFlags,
        out: &mut Vec<io::Result<rustix::fs::Statx>>,
    ) {
        use rustix::fd::AsRawFd;

        self.paths.clear();
        self.offsets.clear();
        for name in names {
            self.offsets.push(self.paths.len());
            self.paths.extend_from_slice(name);
            self.paths.push(0);
        }
        let n = self.offsets.len();
        out.clear();
        out.resize_with(n, || Err(io::Errno::CANCELED));
        self.bufs.clear();
        self.bufs.resize_with(n, std::mem::MaybeUninit::uninit);
        let capacity = self.ring.params().sq_entries() as usize;
        let fd = io_uring::types::Fd(dir.as_raw_fd());
        let mut done = vec![false; n];
        let (mut next, mut in_flight) = (0usize, 0usize);
        if self.broken {
            for (slot, name) in out
                .iter_mut()
                .zip(self.offsets.iter().map(|&o| &self.paths[o..]))
            {
                let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
                *slot = rustix::fs::statx(dir, &name[..len], flags, mask);
            }
            return;
        }
        while next < n || in_flight > 0 {
            {
                let mut sq = self.ring.submission();
                while next < n && in_flight < capacity {
                    let entry = io_uring::opcode::Statx::new(
                        fd,
                        self.paths[self.offsets[next]..].as_ptr().cast(),
                        self.bufs[next].as_mut_ptr().cast(),
                    )
                    .flags(flags.bits() as i32)
                    .mask(mask.bits())
                    .build()
                    .user_data(next as u64);
                    // SAFETY: the name is NUL-terminated inside `paths` and
                    // the output buffer is `bufs[next]`, a `struct statx`
                    // (size asserted above). Neither vector is touched until
                    // every submitted entry has completed: this loop only
                    // ends with `in_flight == 0`, and if the ring breaks the
                    // vectors are leaked instead (see below). `dir` outlives
                    // the call. `in_flight < capacity` keeps the queue from
                    // overflowing, so `push` cannot fail.
                    if unsafe { sq.push(&entry) }.is_err() {
                        break;
                    }
                    next += 1;
                    in_flight += 1;
                }
            }
            match self.ring.submit_and_wait(1) {
                Ok(_) => {}
                Err(e)
                    if matches!(
                        io::Errno::from_io_error(&e),
                        Some(io::Errno::INTR | io::Errno::AGAIN | io::Errno::BUSY)
                    ) => {}
                Err(_) => {
                    // Entries may still be running in the kernel: leak the
                    // buffers they point to rather than reuse them, and
                    // finish the batch from a copy of the names.
                    self.broken = true;
                    let names = self.paths.clone();
                    std::mem::forget(std::mem::take(&mut self.paths));
                    std::mem::forget(std::mem::take(&mut self.bufs));
                    for (i, ok) in done.iter().enumerate() {
                        if !ok {
                            let name = &names[self.offsets[i]..];
                            let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
                            out[i] = rustix::fs::statx(dir, &name[..len], flags, mask);
                        }
                    }
                    return;
                }
            }
            for cqe in self.ring.completion() {
                let i = cqe.user_data() as usize;
                if i >= n || done[i] {
                    continue;
                }
                done[i] = true;
                in_flight -= 1;
                let res = cqe.result();
                out[i] = if res < 0 {
                    Err(io::Errno::from_raw_os_error(-res))
                } else {
                    // SAFETY: a non-negative result means the kernel filled
                    // `bufs[i]` with a `struct statx`, which is plain data.
                    Ok(unsafe { self.bufs[i].assume_init_read() })
                };
            }
        }
    }
}
