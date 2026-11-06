// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Per-node flag bits (F0-diseno.md §1.4).

use core::ops::{BitAnd, BitOr, BitOrAssign};

/// Per-node flag bits (F0-diseno.md §1.4). Bit 15 is reserved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Flags(pub u16);

impl Flags {
    /// No flag set.
    pub const EMPTY: Flags = Flags(0);
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

    /// Flags whose node never contributes its sizes to its parent
    /// (invariant I4). A change of device also stops the contribution; see
    /// [`crate::Arena::contributes`].
    pub const NON_CONTRIBUTING: Flags =
        Flags(Self::HARDLINK_DUP.0 | Self::MOUNT_POINT.0 | Self::BIND_ALIAS.0);

    /// Raw bits.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether every bit of `other` is set.
    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any bit of `other` is set.
    pub const fn intersects(self, other: Flags) -> bool {
        self.0 & other.0 != 0
    }

    /// Sets the bits of `other`.
    pub fn insert(&mut self, other: Flags) {
        self.0 |= other.0;
    }
}

impl BitOr for Flags {
    type Output = Flags;
    fn bitor(self, rhs: Flags) -> Flags {
        Flags(self.0 | rhs.0)
    }
}

impl BitOrAssign for Flags {
    fn bitor_assign(&mut self, rhs: Flags) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Flags {
    type Output = Flags;
    fn bitand(self, rhs: Flags) -> Flags {
        Flags(self.0 & rhs.0)
    }
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
        let union = ALL.iter().fold(Flags::EMPTY, |acc, &f| acc | f);
        assert_eq!(union.bits(), 0x7FFF);
    }

    #[test]
    fn set_operations() {
        let mut f = Flags::DIR;
        f.insert(Flags::LOCKED);
        assert!(f.contains(Flags::DIR | Flags::LOCKED));
        assert!(!f.contains(Flags::DIR | Flags::ERROR));
        assert!(f.intersects(Flags::LOCKED | Flags::ERROR));
        assert!(!f.intersects(Flags::NON_CONTRIBUTING));
    }
}
