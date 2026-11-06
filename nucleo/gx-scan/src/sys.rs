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
