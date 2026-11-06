// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Read-only arena (F0-diseno.md §1, §8).

use std::collections::HashMap;
use std::ops::Range;

use crate::class::Class;
use crate::column::Frozen;
use crate::flags::Flags;
use crate::tables::{Device, DirRow, ExtTable, OWNER_OVERFLOW, Summary};
use crate::{NodeId, path_from_chain};

pub(crate) struct NodeColumns {
    pub parent: Frozen<u32>,
    pub name_off: Frozen<u32>,
    pub name_len: Frozen<u8>,
    pub class: Frozen<u8>,
    pub flags: Frozen<u16>,
    pub aux: Frozen<u32>,
    pub disk_bytes: Frozen<u64>,
    pub ino: Frozen<u64>,
    pub apparent_bytes: Frozen<u64>,
    pub mtime: Frozen<u32>,
    pub dev: Frozen<u16>,
    pub mode: Frozen<u16>,
    pub owner: Frozen<u16>,
}

pub(crate) struct Tables {
    pub names: Frozen<u8>,
    pub dirs: Frozen<DirRow>,
    pub summaries: Vec<Summary>,
    pub summary_nodes: Vec<u32>,
    pub devices: Vec<Device>,
    pub device_strings: Vec<u8>,
    pub owners: Vec<(u32, u32)>,
    pub owner_overflow: HashMap<u32, (u32, u32)>,
    pub exts: ExtTable,
    pub root_name_len: u32,
    pub node_capacity: u32,
}

/// First invariant violation found by [`Arena::check_invariants`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvariantError {
    /// The arena has no nodes.
    #[error("the arena is empty")]
    Empty,
    /// Node columns have different lengths.
    #[error("column {column} has length {len}, expected {expected}")]
    ColumnLength {
        /// Column name.
        column: &'static str,
        /// Its length.
        len: usize,
        /// Number of nodes.
        expected: usize,
    },
    /// I1: the root's parent is not `NONE` or the root is not a directory.
    #[error("I1: invalid root ({0})")]
    Root(&'static str),
    /// I2: a directory's child range is invalid.
    #[error("I2: directory {dir} has invalid child range {first}+{count}")]
    ChildRange {
        /// Directory node.
        dir: u32,
        /// `first_child`.
        first: u32,
        /// `child_count`.
        count: u32,
    },
    /// A directory's `aux` does not point to its own row.
    #[error("directory {dir} has aux {aux}, which is not its directory row")]
    DirRow {
        /// Directory node.
        dir: u32,
        /// Its `aux` value.
        aux: u32,
    },
    /// I3: a node's parent is invalid or the node is outside its range.
    #[error("I3: node {node} has invalid parent {parent} ({reason})")]
    Parent {
        /// Node.
        node: u32,
        /// Its parent value.
        parent: u32,
        /// What is wrong.
        reason: &'static str,
    },
    /// I4/I5: a directory total is smaller than what its children contribute.
    #[error("I4: node {node} {kind} total {total} is below its children's {children}")]
    Total {
        /// Directory node.
        node: u32,
        /// `"disk"` or `"apparent"`.
        kind: &'static str,
        /// Stored total.
        total: u64,
        /// Sum of contributing children.
        children: u64,
    },
    /// I5: a `HARDLINK_DUP` node is a directory.
    #[error("I5: node {0} is a directory marked HARDLINK_DUP")]
    HardlinkDir(u32),
    /// I6: a name is out of bounds, has an invalid length or contains `/` or NUL.
    #[error("I6: node {node} has an invalid name ({reason})")]
    Name {
        /// Node.
        node: u32,
        /// What is wrong.
        reason: &'static str,
    },
}

/// Bytes used by the arena's storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryUsage {
    /// Hot columns (24 B per node).
    pub hot: u64,
    /// Cold columns (26 B per node).
    pub cold: u64,
    /// Name buffer.
    pub names: u64,
    /// Directory table.
    pub dirs: u64,
    /// Summaries.
    pub summaries: u64,
}

impl MemoryUsage {
    /// Sum of all parts.
    pub fn total(&self) -> u64 {
        self.hot + self.cold + self.names + self.dirs + self.summaries
    }
}

/// Read-only, shareable arena of nodes.
///
/// Accessors taking a [`NodeId`] panic (bounds-checked) if it is not below
/// [`Arena::len`].
pub struct Arena {
    c: NodeColumns,
    t: Tables,
}

impl Arena {
    pub(crate) fn from_parts(c: NodeColumns, t: Tables) -> Arena {
        Arena { c, t }
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.c.parent.len()
    }

    /// Whether the arena has no nodes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The root node (always 0).
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// Parent of `n`, `None` for the root.
    pub fn parent(&self, n: NodeId) -> Option<NodeId> {
        let p = self.c.parent[n.0 as usize];
        (p != NodeId::NONE.0).then_some(NodeId(p))
    }

    /// Children of `n` as an id range (empty if `n` is not a directory).
    pub fn children(&self, n: NodeId) -> Range<u32> {
        if !self.flags(n).contains(Flags::DIR) {
            return 0..0;
        }
        match self.t.dirs.get(self.c.aux[n.0 as usize] as usize) {
            Some(row) => row.first_child..row.first_child.saturating_add(row.child_count),
            None => 0..0,
        }
    }

    /// Name of `n`; for the root, the absolute scanned path.
    pub fn name(&self, n: NodeId) -> &[u8] {
        let i = n.0 as usize;
        let off = self.c.name_off[i] as usize;
        let len = if i == 0 {
            self.t.root_name_len as usize
        } else {
            self.c.name_len[i] as usize
        };
        &self.t.names[off..off + len]
    }

    /// Flags of `n`.
    pub fn flags(&self, n: NodeId) -> Flags {
        Flags(self.c.flags[n.0 as usize])
    }

    /// Class of `n`.
    pub fn class(&self, n: NodeId) -> Class {
        Class::from_u8(self.c.class[n.0 as usize])
    }

    /// Disk usage of `n` (for directories, the total per I4).
    pub fn disk_bytes(&self, n: NodeId) -> u64 {
        self.c.disk_bytes[n.0 as usize]
    }

    /// Apparent size of `n` (for directories, the total per I4).
    pub fn apparent_bytes(&self, n: NodeId) -> u64 {
        self.c.apparent_bytes[n.0 as usize]
    }

    /// Writes the absolute path of `n` into `out` (cleared first).
    pub fn path(&self, n: NodeId, out: &mut Vec<u8>) {
        assert!((n.0 as usize) < self.len(), "node {} out of range", n.0);
        path_from_chain(
            n.0,
            |x| self.c.parent[x as usize],
            |x, buf: &mut Vec<u8>| buf.extend_from_slice(self.name(NodeId(x))),
            self.name(self.root()),
            out,
        );
    }

    /// `stx_ino` of `n`.
    pub fn ino(&self, n: NodeId) -> u64 {
        self.c.ino[n.0 as usize]
    }

    /// Modification time of `n` (Unix seconds, saturated).
    pub fn mtime(&self, n: NodeId) -> u32 {
        self.c.mtime[n.0 as usize]
    }

    /// Device index of `n`.
    pub fn dev(&self, n: NodeId) -> u16 {
        self.c.dev[n.0 as usize]
    }

    /// `stx_mode` of `n`.
    pub fn mode(&self, n: NodeId) -> u16 {
        self.c.mode[n.0 as usize]
    }

    /// `(uid, gid)` of `n`.
    pub fn owner(&self, n: NodeId) -> (u32, u32) {
        let i = self.c.owner[n.0 as usize];
        if i == OWNER_OVERFLOW {
            return self.t.owner_overflow.get(&n.0).copied().unwrap_or((0, 0));
        }
        self.t.owners.get(i as usize).copied().unwrap_or((0, 0))
    }

    /// Lowercase extension of a non-directory node (empty if none).
    pub fn extension(&self, n: NodeId) -> &[u8] {
        if self.flags(n).contains(Flags::DIR) {
            return &[];
        }
        self.t.exts.get(self.c.aux[n.0 as usize])
    }

    /// Whether `n` adds its sizes to its parent (I4): not the root, not
    /// `HARDLINK_DUP`, `MOUNT_POINT` or `BIND_ALIAS`, and on the same device
    /// as its parent.
    pub fn contributes(&self, n: NodeId) -> bool {
        match self.parent(n) {
            None => false,
            Some(p) => {
                !self.flags(n).intersects(Flags::NON_CONTRIBUTING) && self.dev(n) == self.dev(p)
            }
        }
    }

    /// Device table.
    pub fn devices(&self) -> &[Device] {
        &self.t.devices
    }

    /// Mount path of a device row.
    pub fn device_mount_path(&self, d: &Device) -> &[u8] {
        let off = d.mount_path_off as usize;
        self.t
            .device_strings
            .get(off..off + d.mount_path_len as usize)
            .unwrap_or(&[])
    }

    /// Number of directories.
    pub fn dir_count(&self) -> usize {
        self.t.dirs.len()
    }

    /// Summary of a directory with at least 1 024 children.
    pub fn summary(&self, n: NodeId) -> Option<&Summary> {
        let i = self.t.summary_nodes.binary_search(&n.0).ok()?;
        self.t.summaries.get(i)
    }

    /// Number of summaries.
    pub fn summary_count(&self) -> usize {
        self.t.summaries.len()
    }

    /// Node capacity that was reserved while building.
    pub fn node_capacity(&self) -> u32 {
        self.t.node_capacity
    }

    /// Bytes used by columns, names and tables.
    pub fn memory_usage(&self) -> MemoryUsage {
        let n = self.len() as u64;
        MemoryUsage {
            hot: n * 24,
            cold: n * 26,
            names: self.t.names.len() as u64,
            dirs: self.t.dirs.len() as u64 * size_of::<DirRow>() as u64,
            summaries: self.t.summaries.len() as u64 * size_of::<Summary>() as u64,
        }
    }

    /// Verifies invariants I1 to I6 and returns the first violation.
    ///
    /// I4 is checked as "each directory total is at least the sum of what
    /// its children contribute": the arena does not keep a directory's own
    /// blocks separately, so exact totals are checked against `du` and an
    /// independent model in the tests instead.
    pub fn check_invariants(&self) -> Result<(), InvariantError> {
        let n = self.len();
        if n == 0 {
            return Err(InvariantError::Empty);
        }
        self.check_lengths(n)?;

        // I1.
        if self.c.parent[0] != NodeId::NONE.0 {
            return Err(InvariantError::Root("parent is not NONE"));
        }
        if !self.flags(self.root()).contains(Flags::DIR) {
            return Err(InvariantError::Root("not a directory"));
        }

        // I6.
        let names = self.t.names.len();
        for i in 0..n {
            let off = self.c.name_off[i] as usize;
            let len = if i == 0 {
                self.t.root_name_len as usize
            } else {
                self.c.name_len[i] as usize
            };
            let Some(name) = off
                .checked_add(len)
                .and_then(|end| self.t.names.get(off..end))
            else {
                return Err(InvariantError::Name {
                    node: i as u32,
                    reason: "out of bounds",
                });
            };
            debug_assert!(off + len <= names);
            if i == 0 {
                if name.is_empty() || name[0] != b'/' || name.contains(&0) {
                    return Err(InvariantError::Root("name is not an absolute path"));
                }
            } else if len == 0 {
                return Err(InvariantError::Name {
                    node: i as u32,
                    reason: "empty",
                });
            } else if name.iter().any(|&b| b == b'/' || b == 0) {
                return Err(InvariantError::Name {
                    node: i as u32,
                    reason: "contains '/' or NUL",
                });
            }
        }

        // I2: every directory's range is in bounds and its children point back.
        for d in 0..n as u32 {
            let flags = self.flags(NodeId(d));
            if !flags.contains(Flags::DIR) {
                continue;
            }
            let aux = self.c.aux[d as usize];
            let Some(row) = self.t.dirs.get(aux as usize) else {
                return Err(InvariantError::DirRow { dir: d, aux });
            };
            if row.node != d {
                return Err(InvariantError::DirRow { dir: d, aux });
            }
            let bad = InvariantError::ChildRange {
                dir: d,
                first: row.first_child,
                count: row.child_count,
            };
            let end = row
                .first_child
                .checked_add(row.child_count)
                .ok_or(bad.clone())?;
            if end as usize > n || (row.child_count > 0 && row.first_child <= d) {
                return Err(bad);
            }
            for c in row.first_child..end {
                if self.c.parent[c as usize] != d {
                    return Err(InvariantError::Parent {
                        node: c,
                        parent: self.c.parent[c as usize],
                        reason: "child range contains a node with another parent",
                    });
                }
            }
        }

        // I3: every non-root node has a directory parent whose range contains it.
        for i in 1..n as u32 {
            let p = self.c.parent[i as usize];
            let reason = if p as usize >= n {
                Some("out of range")
            } else if !self.flags(NodeId(p)).contains(Flags::DIR) {
                Some("not a directory")
            } else if !self.children(NodeId(p)).contains(&i) {
                Some("node outside the parent's child range")
            } else {
                None
            };
            if let Some(reason) = reason {
                return Err(InvariantError::Parent {
                    node: i,
                    parent: p,
                    reason,
                });
            }
            let flags = self.flags(NodeId(i));
            if flags.contains(Flags::DIR | Flags::HARDLINK_DUP) {
                return Err(InvariantError::HardlinkDir(i));
            }
        }

        // I4 / I5.
        let mut disk = vec![0u64; n];
        let mut apparent = vec![0u64; n];
        for i in (1..n).rev() {
            if self.contributes(NodeId(i as u32)) {
                let p = self.c.parent[i] as usize;
                disk[p] = disk[p].saturating_add(self.c.disk_bytes[i]);
                apparent[p] = apparent[p].saturating_add(self.c.apparent_bytes[i]);
            }
        }
        for i in 0..n {
            for (kind, total, children) in [
                ("disk", self.c.disk_bytes[i], disk[i]),
                ("apparent", self.c.apparent_bytes[i], apparent[i]),
            ] {
                if total < children {
                    return Err(InvariantError::Total {
                        node: i as u32,
                        kind,
                        total,
                        children,
                    });
                }
            }
        }
        Ok(())
    }

    fn check_lengths(&self, n: usize) -> Result<(), InvariantError> {
        let c = &self.c;
        let lens = [
            ("name_off", c.name_off.len()),
            ("name_len", c.name_len.len()),
            ("class", c.class.len()),
            ("flags", c.flags.len()),
            ("aux", c.aux.len()),
            ("disk_bytes", c.disk_bytes.len()),
            ("ino", c.ino.len()),
            ("apparent_bytes", c.apparent_bytes.len()),
            ("mtime", c.mtime.len()),
            ("dev", c.dev.len()),
            ("mode", c.mode.len()),
            ("owner", c.owner.len()),
        ];
        for (column, len) in lens {
            if len != n {
                return Err(InvariantError::ColumnLength {
                    column,
                    len,
                    expected: n,
                });
            }
        }
        Ok(())
    }
}

impl core::fmt::Debug for Arena {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Arena")
            .field("nodes", &self.len())
            .field("dirs", &self.dir_count())
            .field("devices", &self.t.devices.len())
            .finish_non_exhaustive()
    }
}
