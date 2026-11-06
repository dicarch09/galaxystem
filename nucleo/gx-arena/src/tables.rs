// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Auxiliary tables (F0-diseno.md §1.6).

use std::collections::HashMap;

use crate::class::Class;

/// Row of the directory table (12 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct DirRow {
    /// Node id of the directory.
    pub node: u32,
    /// First child id.
    pub first_child: u32,
    /// Number of children.
    pub child_count: u32,
}

/// Per-class summary of the direct children of a large directory (96 bytes).
///
/// Only non-directory children are counted; `bytes` holds what each child
/// contributes to its parent (so `HARDLINK_DUP` children add 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Summary {
    /// Number of children per class, indexed by `Class as usize`.
    pub count: [u32; Class::COUNT],
    /// Disk bytes per class, indexed by `Class as usize`.
    pub bytes: [u64; Class::COUNT],
}

/// Directories with at least this many children get a [`Summary`].
pub const SUMMARY_MIN_CHILDREN: u32 = 1024;

/// Row of the device table: one per distinct `st_dev` seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Device {
    /// `makedev(stx_dev_major, stx_dev_minor)`.
    pub dev: u64,
    /// `stx_mnt_id` of the device root.
    pub mnt_id: u64,
    /// `statfs.f_type`.
    pub fs_magic: u64,
    /// 0 = not rotational, 1 = rotational, 2 = unknown.
    pub rotational: u8,
    /// Root node of this galaxy.
    pub mount_node: u32,
    /// btrfs generation (0 if not applicable).
    pub generation: u64,
    /// Offset of the mount path in the device string buffer.
    pub mount_path_off: u32,
    /// Length of the mount path.
    pub mount_path_len: u32,
}

/// Owner index meaning "look up the side map".
pub const OWNER_OVERFLOW: u16 = 0xFFFF;

/// Interning table of `(uid, gid)` pairs.
#[derive(Debug, Default)]
pub struct OwnerTable {
    pairs: Vec<(u32, u32)>,
    index: HashMap<(u32, u32), u16>,
}

impl OwnerTable {
    /// Returns the index of `(uid, gid)`, or [`OWNER_OVERFLOW`] if the table
    /// is full.
    pub fn intern(&mut self, uid: u32, gid: u32) -> u16 {
        if let Some(&i) = self.index.get(&(uid, gid)) {
            return i;
        }
        if self.pairs.len() >= OWNER_OVERFLOW as usize {
            return OWNER_OVERFLOW;
        }
        let i = self.pairs.len() as u16;
        self.pairs.push((uid, gid));
        self.index.insert((uid, gid), i);
        i
    }

    /// All interned pairs.
    pub fn pairs(&self) -> &[(u32, u32)] {
        &self.pairs
    }
}

/// Interning table of lowercase extensions. Id 0 means "no extension".
#[derive(Debug)]
pub struct ExtTable {
    exts: Vec<Box<[u8]>>,
    classes: Vec<Class>,
    index: HashMap<Box<[u8]>, u32>,
}

impl Default for ExtTable {
    fn default() -> Self {
        ExtTable {
            exts: vec![Box::default()],
            classes: vec![Class::Other],
            index: HashMap::new(),
        }
    }
}

impl ExtTable {
    /// Interns an extension (any case) and returns its id and class.
    pub fn intern(&mut self, ext: &[u8]) -> (u32, Class) {
        let lower: Box<[u8]> = ext.to_ascii_lowercase().into_boxed_slice();
        if let Some(&id) = self.index.get(&lower) {
            return (id, self.classes[id as usize]);
        }
        let class = Class::from_extension(&lower);
        let id = self.exts.len() as u32;
        self.exts.push(lower.clone());
        self.classes.push(class);
        self.index.insert(lower, id);
        (id, class)
    }

    /// Extension bytes for `id` (empty for 0 or unknown ids).
    pub fn get(&self, id: u32) -> &[u8] {
        self.exts.get(id as usize).map_or(&[], |e| e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_sizes_match_design() {
        assert_eq!(size_of::<DirRow>(), 12);
        assert_eq!(size_of::<Summary>(), 96);
    }

    #[test]
    fn interning() {
        let mut owners = OwnerTable::default();
        assert_eq!(owners.intern(1000, 1000), 0);
        assert_eq!(owners.intern(0, 0), 1);
        assert_eq!(owners.intern(1000, 1000), 0);

        let mut exts = ExtTable::default();
        let (png, class) = exts.intern(b"PNG");
        assert_eq!(class, Class::Image);
        assert_eq!(exts.intern(b"png").0, png);
        assert_eq!(exts.get(png), b"png");
        assert_eq!(exts.get(0), b"");
    }

    #[test]
    fn owner_overflow() {
        let mut owners = OwnerTable::default();
        for uid in 0..OWNER_OVERFLOW as u32 {
            assert_ne!(owners.intern(uid, 0), OWNER_OVERFLOW);
        }
        assert_eq!(owners.intern(u32::MAX, 0), OWNER_OVERFLOW);
        assert_eq!(owners.intern(0, 0), 0);
    }
}
