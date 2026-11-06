// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writable arena used while scanning.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Mutex, MutexGuard, PoisonError};

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

#[derive(Default)]
struct DeviceTable {
    rows: Vec<Device>,
    strings: Vec<u8>,
    index: HashMap<u64, u16>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding one of these locks leaves plain data behind;
    // the scan is aborted anyway, so poisoning carries no information.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Per-thread cache in front of the shared extension and owner tables, so
/// that interning does not take a lock for every entry.
#[derive(Default)]
pub struct LocalCache {
    exts: HashMap<Box<[u8]>, (u32, Class)>,
    owners: HashMap<(u32, u32), u16>,
}

/// Writable arena. Built by the scanner, then turned into an [`Arena`] with
/// [`ArenaBuilder::freeze`].
///
/// All insertion methods take `&self`, so the builder can be shared between
/// threads. Each node and each directory row must be written by one thread
/// at a time; the scanner guarantees this (F0-diseno.md §2.2).
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
    devices: Mutex<DeviceTable>,
    owners: Mutex<OwnerTable>,
    owner_overflow: Mutex<HashMap<u32, (u32, u32)>>,
    exts: Mutex<ExtTable>,
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
            devices: Mutex::new(DeviceTable::default()),
            owners: Mutex::new(OwnerTable::default()),
            owner_overflow: Mutex::new(HashMap::new()),
            exts: Mutex::new(ExtTable::default()),
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
        self.write_node(
            ids.start,
            NodeId::NONE.0,
            off,
            0,
            record,
            &mut LocalCache::default(),
        )?;
        self.root_path = path.to_vec();
        Ok(NodeId(ids.start))
    }

    /// Inserts the children of `parent` as one contiguous block of ids and
    /// returns their range (invariant I2). Must be called at most once per
    /// directory. The block is reserved with a single atomic operation, so
    /// concurrent calls for different directories never interleave ids.
    pub fn add_children(
        &self,
        parent: NodeId,
        children: &[NodeRecord<'_>],
        cache: &mut LocalCache,
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
            self.write_node(id, p, off, child.name.len() as u8, child, cache)?;
            off += child.name.len() as u32;
        }
        row.first_child = ids.start;
        row.child_count = n;
        self.dirs.set(row_index, row)?;
        Ok(ids)
    }

    fn reserve_nodes(&self, n: u32) -> Result<Range<u32>, ArenaError> {
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
        &self,
        id: u32,
        parent: u32,
        name_off: u32,
        name_len: u8,
        r: &NodeRecord<'_>,
        cache: &mut LocalCache,
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
                Some(ext) if id != 0 => self.intern_ext(ext, cache),
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
        let owner = match cache.owners.get(&(r.uid, r.gid)) {
            Some(&o) => o,
            None => {
                let o = lock(&self.owners).intern(r.uid, r.gid);
                cache.owners.insert((r.uid, r.gid), o);
                o
            }
        };
        if owner == OWNER_OVERFLOW {
            lock(&self.owner_overflow).insert(id, (r.uid, r.gid));
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

    fn intern_ext(&self, ext: &[u8], cache: &mut LocalCache) -> (u32, Class) {
        // Extensions are at most MAX_EXTENSION_LEN bytes: lowercase on the stack.
        let mut buf = [0u8; crate::class::MAX_EXTENSION_LEN];
        let lower = &mut buf[..ext.len()];
        lower.copy_from_slice(ext);
        lower.make_ascii_lowercase();
        if let Some(&hit) = cache.exts.get(&*lower) {
            return hit;
        }
        let v = lock(&self.exts).intern(lower);
        cache.exts.insert(lower.into(), v);
        v
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
    pub fn add_flags(&self, n: NodeId, extra: Flags) -> Result<(), ArenaError> {
        self.flags.fetch_or(n.0, extra.bits())?;
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

    /// Current apparent bytes of `n`.
    pub fn apparent_bytes(&self, n: NodeId) -> Result<u64, ArenaError> {
        self.apparent_bytes
            .get(n.0)
            .ok_or(ArenaError::UnknownNode(n.0))
    }

    /// Overwrites the sizes of `n` (used to store aggregated totals).
    pub fn set_sizes(&self, n: NodeId, disk: u64, apparent: u64) -> Result<(), ArenaError> {
        self.disk_bytes.set(n.0, disk)?;
        self.apparent_bytes.set(n.0, apparent)?;
        Ok(())
    }

    /// Atomically adds `disk` and `apparent` to the sizes of `n`. Used by the
    /// concurrent aggregation of F0-diseno.md §2.3, where the sizes of a
    /// directory grow from its own blocks to its total while it is scanned.
    pub fn add_sizes(&self, n: NodeId, disk: u64, apparent: u64) -> Result<(), ArenaError> {
        if disk != 0 {
            self.disk_bytes.fetch_add(n.0, disk)?;
        }
        if apparent != 0 {
            self.apparent_bytes.fetch_add(n.0, apparent)?;
        }
        Ok(())
    }

    /// Directory-table row index of directory `n`.
    pub fn dir_index(&self, n: NodeId) -> Result<u32, ArenaError> {
        if !self.flags_raw(n.0)?.contains(Flags::DIR) {
            return Err(ArenaError::NotADirectory(n.0));
        }
        self.aux.get(n.0).ok_or(ArenaError::UnknownNode(n.0))
    }

    /// Number of directory rows reserved so far.
    pub fn dir_count(&self) -> u32 {
        self.dirs.len()
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
        lock(&self.devices).index.get(&dev).copied()
    }

    /// Registers a device row with its mount path and returns its index.
    /// Registers a device row with its mount path and returns its index and
    /// whether it is new. If another thread registered the same device
    /// first, its row is kept.
    pub fn add_device(
        &self,
        mut device: Device,
        mount_path: &[u8],
    ) -> Result<(u16, bool), ArenaError> {
        let mut t = lock(&self.devices);
        if let Some(&i) = t.index.get(&device.dev) {
            return Ok((i, false));
        }
        let index = u16::try_from(t.rows.len()).map_err(|_| ArenaError::TooManyDevices)?;
        device.mount_path_off = t.strings.len() as u32;
        device.mount_path_len = mount_path.len() as u32;
        t.strings.extend_from_slice(mount_path);
        t.rows.push(device);
        t.index.insert(device.dev, index);
        Ok((index, true))
    }

    /// Updates a device row in place.
    pub fn update_device(&self, index: u16, f: impl FnOnce(&mut Device)) {
        if let Some(d) = lock(&self.devices).rows.get_mut(index as usize) {
            f(d);
        }
    }

    /// Copy of a device row.
    pub fn device(&self, index: u16) -> Option<Device> {
        lock(&self.devices).rows.get(index as usize).copied()
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
        let devices = self
            .devices
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
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
                devices: devices.rows,
                device_strings: devices.strings,
                owners: self
                    .owners
                    .into_inner()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pairs()
                    .to_vec(),
                owner_overflow: self
                    .owner_overflow
                    .into_inner()
                    .unwrap_or_else(PoisonError::into_inner),
                exts: self
                    .exts
                    .into_inner()
                    .unwrap_or_else(PoisonError::into_inner),
                root_name_len: self.root_path.len() as u32,
                node_capacity: self.node_capacity,
            },
        )
    }
}
