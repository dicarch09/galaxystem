// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Node arena: core types and columnar storage (F0-diseno.md §1).
//!
//! F0.0 only defines [`NodeId`] and [`Flags`]; storage arrives in F0.1.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

/// Identifier of a node: its index in every arena column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    /// Sentinel meaning "no node"; the parent of the root (invariant I1).
    pub const NONE: NodeId = NodeId(u32::MAX);
}

/// Per-node flag bits (F0-diseno.md §1.4). Bit 15 is reserved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Flags(pub u16);

impl Flags {
    /// Directory.
    pub const DIR: Flags = Flags(1 << 0);
    /// Symbolic link (never followed).
    pub const SYMLINK: Flags = Flags(1 << 1);
    /// Device, socket or fifo.
    pub const SPECIAL: Flags = Flags(1 << 2);
    /// Root of another filesystem.
    pub const MOUNT_POINT: Flags = Flags(1 << 3);
    /// proc, sysfs and similar; contents are not scanned.
    pub const PSEUDO_FS: Flags = Flags(1 << 4);
    /// NFS, CIFS, sshfs or another network FUSE filesystem.
    pub const NET_FS: Flags = Flags(1 << 5);
    /// The user cannot read it (`EACCES` on open).
    pub const LOCKED: Flags = Flags(1 << 6);
    /// Repeated appearance of a `(dev, ino)` with `nlink > 1`.
    pub const HARDLINK_DUP: Flags = Flags(1 << 7);
    /// `st_blocks * 512 < st_size`.
    pub const SPARSE: Flags = Flags(1 << 8);
    /// Root of a btrfs subvolume (inode 256).
    pub const SUBVOL: Flags = Flags(1 << 9);
    /// Subvolume with a non-zero `parent_uuid` (a snapshot).
    pub const SNAPSHOT_ECHO: Flags = Flags(1 << 10);
    /// The name is not valid UTF-8.
    pub const NON_UTF8: Flags = Flags(1 << 11);
    /// Data may be out of date.
    pub const STALE: Flags = Flags(1 << 12);
    /// Bind mount of something already indexed; not descended.
    pub const BIND_ALIAS: Flags = Flags(1 << 13);
    /// `statx` or open failed for a reason other than `EACCES`.
    pub const ERROR: Flags = Flags(1 << 14);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Flags; 15] = [
        Flags::DIR,
        Flags::SYMLINK,
        Flags::SPECIAL,
        Flags::MOUNT_POINT,
        Flags::PSEUDO_FS,
        Flags::NET_FS,
        Flags::LOCKED,
        Flags::HARDLINK_DUP,
        Flags::SPARSE,
        Flags::SUBVOL,
        Flags::SNAPSHOT_ECHO,
        Flags::NON_UTF8,
        Flags::STALE,
        Flags::BIND_ALIAS,
        Flags::ERROR,
    ];

    #[test]
    fn flags_match_design_bits_and_leave_bit_15_reserved() {
        for (bit, flag) in ALL.iter().enumerate() {
            assert_eq!(flag.0, 1 << bit);
        }
        let union = ALL.iter().fold(0u16, |acc, f| acc | f.0);
        assert_eq!(union, 0x7FFF);
    }

    #[test]
    fn node_id_none_is_u32_max() {
        assert_eq!(NodeId::NONE.0, u32::MAX);
    }
}
