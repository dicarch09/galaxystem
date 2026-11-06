// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writable arena used while scanning.

use std::collections::HashMap;
use std::ops::Range;

use crate::arena::Arena;
use crate::class::{Class, extension};
use crate::column::{Column, ColumnError};
use crate::flags::Flags;
use crate::tables::{
    Device, DirRow, ExtTable, OWNER_OVERFLOW, OwnerTable, SUMMARY_MIN_CHILDREN, Summary,
};
use crate::{NodeId, path_from_chain};

/// Default node capacity: every id a `u32` can hold except [`NodeId::NONE`]
/// (F0-diseno.md §1.8).
pub const DEFAULT_NODE_CAPACITY: u32 = u32::MAX;

/// Smallest capacity the reservation ladder falls back to.
pub const MIN_NODE_CAPACITY: u32 = 1 << 20;

/// Name buffer bytes reserved per node of capacity (virtual memory only).
const NAME_BYTES_PER_NODE: u32 = 64;

const S_IFMT: u16 = 0o170000;
const S_IFREG: u16 = 0o100000;

/// Errors from building an arena.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArenaError {
    /// A column operation failed (capacity exhausted or mmap failure).
    #[error(transparent)]
    Column(#[from] ColumnError),
    /// `set_root` was called on a non-empty arena.
    #[error("the arena already has a root")]
    RootAlreadySet,
    /// The root record is not a directory or its path is invalid.
    #[error("invalid root: {0}")]
    InvalidRoot(&'static str),
    /// The node does not exist.
    #[error("node {0} does not exist")]
    UnknownNode(u32),
    /// Children were added to a node that is not a directory.
    #[error("node {0} is not a directory")]
    NotADirectory(u32),
    /// Children were added twice to the same directory.
    #[error("directory {0} already has its children")]
    ChildrenAlreadySet(u32),
    /// A child name violates invariant I6.
    #[error("invalid child name under node {parent} (length {len})")]
    InvalidName {
        /// Parent directory.
        parent: u32,
        /// Name length.
        len: usize,
    },
    /// More than 65 535 devices.
    #[error("too many devices")]
    TooManyDevices,
    /// `aggregate` was called twice.
    #[error("sizes were already aggregated")]
    AlreadyAggregated,
}

/// Metadata of a node to insert.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeRecord<'a> {
    /// Entry name (ignored for the root).
    pub name: &'a [u8],
    /// Flags, including the type flags (`DIR`, `SYMLINK`, `SPECIAL`).
    pub flags: Flags,
    /// `stx_mode`.
    pub mode: u16,
    /// Own disk usage: `st_blocks * 512`.
    pub disk_bytes: u64,
    /// Own apparent size: `st_size`.
    pub apparent_bytes: u64,
    /// `stx_ino`.
    pub ino: u64,
    /// Modification time, saturated to `u32`.
    pub mtime: u32,
    /// Index in the device table.
    pub dev: u16,
    /// Owner uid.
    pub uid: u32,
    /// Owner gid.
    pub gid: u32,
}

/// Writable arena. Built by the scanner, then turned into an [`Arena`] with
/// [`ArenaBuilder::freeze`].
pub struct ArenaBuilder {
    // Hot columns (§1.2).
    parent: Column<u32>,
    name_off: Column<u32>,
    name_len: Column<u8>,
    class: Column<u8>,
    flags: Column<u16>,
    aux: Column<u32>,
    disk_bytes: Column<u64>,
    // Cold columns (§1.3).
    ino: Column<u64>,
    apparent_bytes: Column<u64>,
    mtime: Column<u32>,
    dev: Column<u16>,
    mode: Column<u16>,
    owner: Column<u16>,
    // Buffers and tables (§1.6).
    names: Column<u8>,
    dirs: Column<DirRow>,
    summaries: Vec<Summary>,
    summary_nodes: Vec<u32>,
    devices: Vec<Device>,
    device_strings: Vec<u8>,
    device_index: HashMap<u64, u16>,
    owners: OwnerTable,
    owner_overflow: HashMap<u32, (u32, u32)>,
    exts: ExtTable,
    root_path: Vec<u8>,
    node_capacity: u32,
    aggregated: bool,
}

impl ArenaBuilder {
    /// Reserves an arena for up to `node_capacity` nodes. If the kernel
    /// refuses the virtual memory, retries with half the capacity until
    /// `min(node_capacity, MIN_NODE_CAPACITY)`; [`Self::node_capacity`]
    /// reports the capacity obtained.
    pub fn new(node_capacity: u32) -> Result<Self, ArenaError> {
        let floor = node_capacity.min(MIN_NODE_CAPACITY);
        let mut capacity = node_capacity;
        loop {
            match Self::with_exact_capacity(capacity) {
                Ok(builder) => return Ok(builder),
                Err(ArenaError::Column(e)) if e.is_out_of_memory() && capacity / 2 >= floor => {
                    capacity /= 2;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Reserves an arena for exactly `node_capacity` nodes, without fallback.
    pub fn with_exact_capacity(node_capacity: u32) -> Result<Self, ArenaError> {
        let names = node_capacity.saturating_mul(NAME_BYTES_PER_NODE);
        Ok(ArenaBuilder {
            parent: Column::with_capacity(node_capacity)?,
            name_off: Column::with_capacity(node_capacity)?,
            name_len: Column::with_capacity(node_capacity)?,
            class: Column::with_capacity(node_capacity)?,
            flags: Column::with_capacity(node_capacity)?,
            aux: Column::with_capacity(node_capacity)?,
            disk_bytes: Column::with_capacity(node_capacity)?,
            ino: Column::with_capacity(node_capacity)?,
            apparent_bytes: Column::with_capacity(node_capacity)?,
            mtime: Column::with_capacity(node_capacity)?,
            dev: Column::with_capacity(node_capacity)?,
            mode: Column::with_capacity(node_capacity)?,
            owner: Column::with_capacity(node_capacity)?,
            names: Column::with_capacity(names)?,
            dirs: Column::with_capacity(node_capacity)?,
            summaries: Vec::new(),
            summary_nodes: Vec::new(),
            devices: Vec::new(),
            device_strings: Vec::new(),
            device_index: HashMap::new(),
            owners: OwnerTable::default(),
            owner_overflow: HashMap::new(),
            exts: ExtTable::default(),
            root_path: Vec::new(),
            node_capacity,
            aggregated: false,
        })
    }

    /// Node capacity actually reserved.
    pub fn node_capacity(&self) -> u32 {
        self.node_capacity
    }

    /// Number of nodes.
    pub fn len(&self) -> u32 {
        self.parent.len()
    }

    /// Whether the arena has no nodes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Inserts the root (node 0). `path` is the absolute scanned path.
    pub fn set_root(&mut self, path: &[u8], record: &NodeRecord<'_>) -> Result<NodeId, ArenaError> {
        if !self.is_empty() {
            return Err(ArenaError::RootAlreadySet);
        }
        if !record.flags.contains(Flags::DIR) {
            return Err(ArenaError::InvalidRoot("not a directory"));
        }
        if path.is_empty() || path.contains(&0) || path[0] != b'/' {
            return Err(ArenaError::InvalidRoot(
                "path must be absolute and NUL-free",
            ));
        }
        let len =
            u32::try_from(path.len()).map_err(|_| ArenaError::InvalidRoot("path too long"))?;
        let off = self.names.reserve(len)?.start;
        self.names.set_slice(off, path)?;
        let ids = self.reserve_nodes(1)?;
        self.write_node(ids.start, NodeId::NONE.0, off, 0, record)?;
        self.root_path = path.to_vec();
        Ok(NodeId(ids.start))
    }

    /// Inserts the children of `parent` as one contiguous block of ids and
    /// returns their range (invariant I2). Must be called at most once per
    /// directory.
    pub fn add_children(
        &mut self,
        parent: NodeId,
        children: &[NodeRecord<'_>],
    ) -> Result<Range<u32>, ArenaError> {
        let p = parent.0;
        let flags = self.flags_raw(p)?;
        if !flags.contains(Flags::DIR) {
            return Err(ArenaError::NotADirectory(p));
        }
        let row_index = self.aux.get(p).ok_or(ArenaError::UnknownNode(p))?;
        let mut row = self.dirs.get(row_index).ok_or(ArenaError::UnknownNode(p))?;
        if row.first_child != 0 {
            return Err(ArenaError::ChildrenAlreadySet(p));
        }
        let mut total_name: u32 = 0;
        for child in children {
            let len = child.name.len();
            if len == 0 || len > 255 || child.name.iter().any(|&b| b == b'/' || b == 0) {
                return Err(ArenaError::InvalidName { parent: p, len });
            }
            total_name =
                total_name
                    .checked_add(len as u32)
                    .ok_or(ColumnError::CapacityExhausted {
                        requested: u32::MAX,
                        len: self.names.len(),
                        capacity: self.names.capacity(),
                    })?;
        }
        let n = u32::try_from(children.len()).map_err(|_| ColumnError::CapacityExhausted {
            requested: u32::MAX,
            len: self.len(),
            capacity: self.node_capacity,
        })?;
        // Names first: if the node reservation then fails, only unused name
        // bytes are left behind, never half-initialised nodes.
        let mut off = self.names.reserve(total_name)?.start;
        let ids = self.reserve_nodes(n)?;
        for (id, child) in ids.clone().zip(children) {
            self.names.set_slice(off, child.name)?;
            self.write_node(id, p, off, child.name.len() as u8, child)?;
            off += child.name.len() as u32;
        }
        row.first_child = ids.start;
        row.child_count = n;
        self.dirs.set(row_index, row)?;
        Ok(ids)
    }

    fn reserve_nodes(&mut self, n: u32) -> Result<Range<u32>, ArenaError> {
        let ids = self.parent.reserve(n)?;
        for col in [&self.name_off, &self.aux, &self.mtime] {
            col.ensure_len(ids.end)?;
        }
        for col in [&self.name_len, &self.class] {
            col.ensure_len(ids.end)?;
        }
        for col in [&self.flags, &self.dev, &self.mode, &self.owner] {
            col.ensure_len(ids.end)?;
        }
        for col in [&self.disk_bytes, &self.ino, &self.apparent_bytes] {
            col.ensure_len(ids.end)?;
        }
        Ok(ids)
    }

    fn write_node(
        &mut self,
        id: u32,
        parent: u32,
        name_off: u32,
        name_len: u8,
        r: &NodeRecord<'_>,
    ) -> Result<(), ArenaError> {
        let (class, aux) = if r.flags.contains(Flags::DIR) {
            let row = self.dirs.push(DirRow {
                node: id,
                first_child: 0,
                child_count: 0,
            })?;
            (Class::Other, row)
        } else {
            let (ext_id, ext_class) = match extension(r.name) {
                Some(ext) if id != 0 => self.exts.intern(ext),
                _ => (0, Class::Other),
            };
            let class = if r.mode & S_IFMT != S_IFREG {
                Class::Other
            } else if ext_id == 0 && r.mode & 0o111 != 0 {
                Class::Binary
            } else {
                ext_class
            };
            (class, ext_id)
        };
        let owner = self.owners.intern(r.uid, r.gid);
        if owner == OWNER_OVERFLOW {
            self.owner_overflow.insert(id, (r.uid, r.gid));
        }
        self.parent.set(id, parent)?;
        self.name_off.set(id, name_off)?;
        self.name_len.set(id, name_len)?;
        self.class.set(id, class as u8)?;
        self.flags.set(id, r.flags.bits())?;
        self.aux.set(id, aux)?;
        self.disk_bytes.set(id, r.disk_bytes)?;
        self.ino.set(id, r.ino)?;
        self.apparent_bytes.set(id, r.apparent_bytes)?;
        self.mtime.set(id, r.mtime)?;
        self.dev.set(id, r.dev)?;
        self.mode.set(id, r.mode)?;
        self.owner.set(id, owner)?;
        Ok(())
    }

    fn flags_raw(&self, n: u32) -> Result<Flags, ArenaError> {
        self.flags
            .get(n)
            .map(Flags)
            .ok_or(ArenaError::UnknownNode(n))
    }

    /// Flags of `n`.
    pub fn flags(&self, n: NodeId) -> Result<Flags, ArenaError> {
        self.flags_raw(n.0)
    }

    /// Adds `extra` to the flags of `n`.
    pub fn add_flags(&mut self, n: NodeId, extra: Flags) -> Result<(), ArenaError> {
        let f = self.flags_raw(n.0)? | extra;
        self.flags.set(n.0, f.bits())?;
        Ok(())
    }

    /// Parent of `n` (`None` for the root).
    pub fn parent(&self, n: NodeId) -> Result<Option<NodeId>, ArenaError> {
        let p = self.parent.get(n.0).ok_or(ArenaError::UnknownNode(n.0))?;
        Ok((p != NodeId::NONE.0).then_some(NodeId(p)))
    }

    /// Device index of `n`.
    pub fn dev(&self, n: NodeId) -> Result<u16, ArenaError> {
        self.dev.get(n.0).ok_or(ArenaError::UnknownNode(n.0))
    }

    /// Current disk bytes of `n` (own until [`Self::aggregate`]).
    pub fn disk_bytes(&self, n: NodeId) -> Result<u64, ArenaError> {
        self.disk_bytes.get(n.0).ok_or(ArenaError::UnknownNode(n.0))
    }

    /// `stx_ino` of `n`.
    pub fn ino(&self, n: NodeId) -> Result<u64, ArenaError> {
        self.ino.get(n.0).ok_or(ArenaError::UnknownNode(n.0))
    }

    /// Writes the absolute path of `n` into `out` (cleared first).
    pub fn path(&self, n: NodeId, out: &mut Vec<u8>) -> Result<(), ArenaError> {
        if n.0 >= self.len() {
            return Err(ArenaError::UnknownNode(n.0));
        }
        path_from_chain(
            n.0,
            |x| self.parent.get(x).unwrap_or(NodeId::NONE.0),
            |x, buf: &mut Vec<u8>| {
                let off = self.name_off.get(x).unwrap_or(0);
                let len = u32::from(self.name_len.get(x).unwrap_or(0));
                buf.extend((off..off + len).map(|i| self.names.get(i).unwrap_or(0)));
            },
            &self.root_path,
            out,
        );
        Ok(())
    }

    /// Index of the device with number `dev`, if registered.
    pub fn device_index(&self, dev: u64) -> Option<u16> {
        self.device_index.get(&dev).copied()
    }

    /// Registers a device row with its mount path and returns its index.
    pub fn add_device(&mut self, mut device: Device, mount_path: &[u8]) -> Result<u16, ArenaError> {
        if let Some(i) = self.device_index(device.dev) {
            return Ok(i);
        }
        let index = u16::try_from(self.devices.len()).map_err(|_| ArenaError::TooManyDevices)?;
        device.mount_path_off = self.device_strings.len() as u32;
        device.mount_path_len = mount_path.len() as u32;
        self.device_strings.extend_from_slice(mount_path);
        self.devices.push(device);
        self.device_index.insert(device.dev, index);
        Ok(index)
    }

    /// Mutable access to a device row.
    pub fn device_mut(&mut self, index: u16) -> Option<&mut Device> {
        self.devices.get_mut(index as usize)
    }

    /// Device row.
    pub fn device(&self, index: u16) -> Option<&Device> {
        self.devices.get(index as usize)
    }

    /// Whether `n` adds its sizes to its parent (invariant I4).
    pub fn contributes(&self, n: NodeId) -> bool {
        let Some(p) = self.parent.get(n.0).filter(|&p| p != NodeId::NONE.0) else {
            return false;
        };
        let flags = Flags(self.flags.get(n.0).unwrap_or(0));
        !flags.intersects(Flags::NON_CONTRIBUTING) && self.dev.get(n.0) == self.dev.get(p)
    }

    /// Post-order aggregation of sizes (I4, I5). Children always have larger
    /// ids than their parent, so a pass in decreasing id order visits every
    /// node after all its descendants.
    pub fn aggregate(&mut self) -> Result<(), ArenaError> {
        if self.aggregated {
            return Err(ArenaError::AlreadyAggregated);
        }
        for n in (1..self.len()).rev() {
            if !self.contributes(NodeId(n)) {
                continue;
            }
            let p = self.parent.get(n).ok_or(ArenaError::UnknownNode(n))?;
            let disk = self.disk_bytes.get(n).unwrap_or(0);
            let apparent = self.apparent_bytes.get(n).unwrap_or(0);
            let pd = self.disk_bytes.get(p).unwrap_or(0);
            let pa = self.apparent_bytes.get(p).unwrap_or(0);
            self.disk_bytes.set(p, pd.saturating_add(disk))?;
            self.apparent_bytes.set(p, pa.saturating_add(apparent))?;
        }
        self.aggregated = true;
        Ok(())
    }

    /// Builds the per-class summaries of directories with at least
    /// [`SUMMARY_MIN_CHILDREN`] children. Call after [`Self::aggregate`].
    pub fn build_summaries(&mut self) {
        self.summaries.clear();
        self.summary_nodes.clear();
        for r in 0..self.dirs.len() {
            let Some(row) = self.dirs.get(r) else {
                continue;
            };
            if row.child_count < SUMMARY_MIN_CHILDREN {
                continue;
            }
            let mut s = Summary::default();
            for c in row.first_child..row.first_child + row.child_count {
                let flags = Flags(self.flags.get(c).unwrap_or(0));
                if flags.contains(Flags::DIR) {
                    continue;
                }
                let class = self.class.get(c).unwrap_or(0) as usize % Class::COUNT;
                s.count[class] += 1;
                if self.contributes(NodeId(c)) {
                    s.bytes[class] += self.disk_bytes.get(c).unwrap_or(0);
                }
            }
            self.summaries.push(s);
            self.summary_nodes.push(row.node);
        }
    }

    /// Freezes the arena into its read-only form.
    pub fn freeze(self) -> Arena {
        Arena::from_parts(
            crate::arena::NodeColumns {
                parent: self.parent.freeze(),
                name_off: self.name_off.freeze(),
                name_len: self.name_len.freeze(),
                class: self.class.freeze(),
                flags: self.flags.freeze(),
                aux: self.aux.freeze(),
                disk_bytes: self.disk_bytes.freeze(),
                ino: self.ino.freeze(),
                apparent_bytes: self.apparent_bytes.freeze(),
                mtime: self.mtime.freeze(),
                dev: self.dev.freeze(),
                mode: self.mode.freeze(),
                owner: self.owner.freeze(),
            },
            crate::arena::Tables {
                names: self.names.freeze(),
                dirs: self.dirs.freeze(),
                summaries: self.summaries,
                summary_nodes: self.summary_nodes,
                devices: self.devices,
                device_strings: self.device_strings,
                owners: self.owners.pairs().to_vec(),
                owner_overflow: self.owner_overflow,
                exts: self.exts,
                root_name_len: self.root_path.len() as u32,
                node_capacity: self.node_capacity,
            },
        )
    }
}
