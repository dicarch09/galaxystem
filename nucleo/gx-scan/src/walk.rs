// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Single-threaded walk (F0-diseno.md §2.1).

use std::collections::{HashSet, VecDeque};
use std::mem::MaybeUninit;
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Instant;

use gx_arena::{Arena, ArenaBuilder, Device, Flags, NodeId, NodeRecord};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, RawDir, ResolveFlags, Statx, StatxFlags};
use rustix::io::Errno;

use crate::mountinfo::MountTable;
use crate::{Progress, ScanError, ScanErrorRecord, ScanOptions, ScanStats, fsmagic, sys};

const MASK: StatxFlags = StatxFlags::TYPE
    .union(StatxFlags::MODE)
    .union(StatxFlags::NLINK)
    .union(StatxFlags::UID)
    .union(StatxFlags::GID)
    .union(StatxFlags::MTIME)
    .union(StatxFlags::INO)
    .union(StatxFlags::SIZE)
    .union(StatxFlags::BLOCKS)
    .union(StatxFlags::MNT_ID);

const FIRST_NODES_MILESTONE: u32 = 10_000;
const BTRFS_SUBVOL_INO: u64 = 256;

/// A directory waiting to be listed.
#[derive(Clone, Copy)]
struct Pending {
    node: NodeId,
    /// `stx_mnt_id`, or 0 when the kernel did not report it.
    mnt_id: u64,
    /// `makedev(stx_dev_major, stx_dev_minor)`.
    devnum: u64,
    /// First directory of its device (root, mount point or subvolume).
    boundary: bool,
    /// On a network filesystem.
    net: bool,
    ino: u64,
}

/// Result of `statx` on one directory entry.
struct Child {
    name: Range<usize>,
    dtype: FileType,
    stat: Result<Statx, Errno>,
}

/// Per-child decisions taken before the ids are known.
#[derive(Clone, Copy, Default)]
struct ChildPlan {
    descend: bool,
    boundary: bool,
    mnt_id: u64,
    devnum: u64,
    net: bool,
    ino: u64,
    multi_link: bool,
    new_device: Option<u16>,
}

struct Walker<'a> {
    opts: &'a ScanOptions,
    progress: &'a dyn Progress,
    b: ArenaBuilder,
    stats: ScanStats,
    queue: VecDeque<Pending>,
    hardlinks: HashSet<(u64, u64)>,
    mounts: MountTable,
    root_mnt: u64,
    has_mnt_id: bool,
    start: Instant,
    last_progress: Instant,
    running_disk: u64,
    buf: Vec<MaybeUninit<u8>>,
    names: Vec<u8>,
    children: Vec<Child>,
    plans: Vec<ChildPlan>,
    dir_path: Vec<u8>,
}

fn devnum(s: &Statx) -> u64 {
    rustix::fs::makedev(s.stx_dev_major, s.stx_dev_minor)
}

fn type_flags(ft: FileType) -> Flags {
    match ft {
        FileType::Directory => Flags::DIR,
        FileType::Symlink => Flags::SYMLINK,
        FileType::RegularFile | FileType::Unknown => Flags::EMPTY,
        _ => Flags::SPECIAL,
    }
}

/// Apparent size as GNU `du --apparent-size` counts it: `st_size` only for
/// regular files and symlinks, 0 for directories and special files.
fn apparent(s: &Statx) -> u64 {
    match FileType::from_raw_mode(s.stx_mode.into()) {
        FileType::RegularFile | FileType::Symlink => s.stx_size,
        _ => 0,
    }
}

fn mtime(s: &Statx) -> u32 {
    s.stx_mtime.tv_sec.clamp(0, i64::from(u32::MAX)) as u32
}

fn raise_nofile_limit() {
    let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile);
    if limit.maximum.is_some() && limit.current != limit.maximum {
        let _ = rustix::process::setrlimit(
            rustix::process::Resource::Nofile,
            rustix::process::Rlimit {
                current: limit.maximum,
                maximum: limit.maximum,
            },
        );
    }
}

pub(crate) fn run(
    root: &Path,
    opts: &ScanOptions,
    progress: &dyn Progress,
) -> Result<(Arena, ScanStats), ScanError> {
    let start = Instant::now();
    raise_nofile_limit();
    let root_err = |source: std::io::Error| ScanError::Root {
        path: root.to_path_buf(),
        source,
    };
    // The user-supplied root is resolved once; everything below it is
    // opened without following symlinks.
    let canon = std::fs::canonicalize(root).map_err(root_err)?;
    let stx = rustix::fs::statx(CWD, &canon, AtFlags::SYMLINK_NOFOLLOW, MASK)
        .map_err(|e| root_err(e.into()))?;
    if FileType::from_raw_mode(stx.stx_mode.into()) != FileType::Directory {
        return Err(ScanError::RootNotDirectory(canon));
    }
    let root_bytes = canon.as_os_str().as_bytes().to_vec();
    let has_mnt_id = StatxFlags::from_bits_retain(stx.stx_mask).contains(StatxFlags::MNT_ID);

    let b = ArenaBuilder::new(opts.node_capacity)?;
    let mut w = Walker {
        opts,
        progress,
        stats: ScanStats {
            root_path: root_bytes.clone(),
            node_capacity: b.node_capacity(),
            ..ScanStats::default()
        },
        b,
        queue: VecDeque::new(),
        hardlinks: HashSet::new(),
        mounts: MountTable::load(),
        root_mnt: stx.stx_mnt_id,
        has_mnt_id,
        start,
        last_progress: start,
        running_disk: 0,
        buf: vec![MaybeUninit::uninit(); opts.getdents_buffer.max(4096)],
        names: Vec::new(),
        children: Vec::new(),
        plans: Vec::new(),
        dir_path: Vec::new(),
    };
    w.add_root(&root_bytes, &stx)?;
    while let Some(p) = w.queue.pop_front() {
        w.visit(p)?;
        w.report_progress();
    }
    w.finish()
}

impl Walker<'_> {
    fn log(&mut self, path: Vec<u8>, op: &'static str, errno: Errno) {
        if self.stats.error_log.len() < self.opts.max_logged_errors {
            self.stats.error_log.push(ScanErrorRecord {
                path,
                op,
                errno: errno.raw_os_error(),
            });
        }
    }

    fn add_root(&mut self, path: &[u8], stx: &Statx) -> Result<(), ScanError> {
        let dn = devnum(stx);
        let dev = self.b.add_device(
            Device {
                dev: dn,
                mnt_id: stx.stx_mnt_id,
                rotational: self.mounts.rotational(stx.stx_mnt_id),
                mount_node: 0,
                ..Device::default()
            },
            path,
        )?;
        let mut flags = Flags::DIR;
        let net = self.has_mnt_id && self.mounts.is_network(stx.stx_mnt_id);
        if net {
            flags |= Flags::NET_FS;
        }
        if std::str::from_utf8(path).is_err() {
            flags |= Flags::NON_UTF8;
        }
        let root = self.b.set_root(
            path,
            &NodeRecord {
                name: b"",
                flags,
                mode: stx.stx_mode,
                disk_bytes: stx.stx_blocks * 512,
                apparent_bytes: apparent(stx),
                ino: stx.stx_ino,
                mtime: mtime(stx),
                dev,
                uid: stx.stx_uid,
                gid: stx.stx_gid,
            },
        )?;
        self.stats.dirs += 1;
        self.queue.push_back(Pending {
            node: root,
            mnt_id: stx.stx_mnt_id,
            devnum: dn,
            boundary: true,
            net,
            ino: stx.stx_ino,
        });
        Ok(())
    }

    fn child_path(&self, name: &[u8]) -> Vec<u8> {
        let mut p = self.dir_path.clone();
        if p.last() != Some(&b'/') {
            p.push(b'/');
        }
        p.extend_from_slice(name);
        p
    }

    fn open_dir(&mut self, p: &Pending) -> Result<Option<OwnedFd>, ScanError> {
        let opened = rustix::fs::openat2(
            CWD,
            self.dir_path.as_slice(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        );
        match opened {
            Ok(fd) => Ok(Some(fd)),
            Err(Errno::ACCESS) => {
                self.b.add_flags(p.node, Flags::LOCKED)?;
                self.stats.locked += 1;
                Ok(None)
            }
            Err(Errno::NOENT) => {
                // Removed between listing its parent and now.
                self.b.add_flags(p.node, Flags::STALE)?;
                self.stats.vanished += 1;
                Ok(None)
            }
            Err(errno) => {
                self.b.add_flags(p.node, Flags::ERROR)?;
                self.stats.errors += 1;
                self.log(self.dir_path.clone(), "openat2", errno);
                Ok(None)
            }
        }
    }

    /// Classifies the first directory of a device. Returns `false` if its
    /// contents must not be listed.
    fn inspect_boundary(&mut self, p: &mut Pending, fd: &OwnedFd) -> Result<bool, ScanError> {
        let dev = self.b.dev(p.node)?;
        let magic = match rustix::fs::fstatfs(fd) {
            // `f_type` is a `long` of a different width on some targets.
            #[allow(clippy::unnecessary_cast)]
            Ok(st) => fsmagic::normalize(st.f_type as i64),
            Err(errno) => {
                self.log(self.dir_path.clone(), "fstatfs", errno);
                0
            }
        };
        if let Some(d) = self.b.device_mut(dev) {
            if d.fs_magic == 0 {
                d.fs_magic = magic;
            }
        }
        if fsmagic::is_pseudo(magic) {
            self.b.add_flags(p.node, Flags::PSEUDO_FS)?;
            self.stats.pseudo_fs += 1;
            return Ok(false);
        }
        if fsmagic::is_network(magic) && !p.net {
            p.net = true;
            self.b.add_flags(p.node, Flags::NET_FS)?;
        }
        if magic == fsmagic::BTRFS && p.ino == BTRFS_SUBVOL_INO {
            let mut extra = Flags::SUBVOL;
            match sys::btrfs_subvol_info(fd) {
                Ok(info) => {
                    if info.parent_uuid != [0; 16] {
                        extra |= Flags::SNAPSHOT_ECHO;
                    }
                    if let Some(d) = self.b.device_mut(dev) {
                        if d.mount_node == p.node.0 {
                            d.generation = info.generation;
                        }
                    }
                }
                Err(errno) => self.log(self.dir_path.clone(), "BTRFS_IOC_GET_SUBVOL_INFO", errno),
            }
            self.b.add_flags(p.node, extra)?;
        }
        Ok(true)
    }

    fn read_entries(&mut self, fd: &OwnedFd, node: NodeId) -> Result<(), ScanError> {
        self.names.clear();
        self.children.clear();
        let mut failed = None;
        {
            let mut dir = RawDir::new(fd, &mut self.buf);
            while let Some(entry) = dir.next() {
                match entry {
                    Ok(e) => {
                        let name = e.file_name().to_bytes();
                        if name == b"." || name == b".." {
                            continue;
                        }
                        let start = self.names.len();
                        self.names.extend_from_slice(name);
                        self.children.push(Child {
                            name: start..self.names.len(),
                            dtype: e.file_type(),
                            stat: Err(Errno::NOENT),
                        });
                    }
                    Err(errno) => {
                        failed = Some(errno);
                        break;
                    }
                }
            }
        }
        if let Some(errno) = failed {
            // Keep what was read; the directory itself is marked.
            self.b.add_flags(node, Flags::ERROR)?;
            self.stats.errors += 1;
            self.log(self.dir_path.clone(), "getdents64", errno);
        }
        Ok(())
    }

    fn visit(&mut self, mut p: Pending) -> Result<(), ScanError> {
        self.b.path(p.node, &mut self.dir_path)?;
        let Some(fd) = self.open_dir(&p)? else {
            return Ok(());
        };
        if p.boundary && !self.inspect_boundary(&mut p, &fd)? {
            return Ok(());
        }
        self.read_entries(&fd, p.node)?;

        let mut at = AtFlags::SYMLINK_NOFOLLOW;
        if p.net {
            at |= AtFlags::STATX_DONT_SYNC;
        }
        for c in &mut self.children {
            c.stat = rustix::fs::statx(&fd, &self.names[c.name.clone()], at, MASK);
        }
        drop(fd);

        // Build the records.
        let children = std::mem::take(&mut self.children);
        let mut records: Vec<NodeRecord<'_>> = Vec::with_capacity(children.len());
        let mut plans = std::mem::take(&mut self.plans);
        plans.clear();
        let names = std::mem::take(&mut self.names);
        for c in &children {
            let name = &names[c.name.clone()];
            let mut plan = ChildPlan::default();
            let record = match &c.stat {
                Err(Errno::NOENT) => {
                    self.stats.vanished += 1;
                    continue;
                }
                Err(errno) => {
                    let errno = *errno;
                    self.stats.errors += 1;
                    let path = self.child_path(name);
                    self.log(path, "statx", errno);
                    NodeRecord {
                        name,
                        flags: type_flags(c.dtype) | Flags::ERROR,
                        dev: self.b.dev(p.node)?,
                        ..NodeRecord::default()
                    }
                }
                Ok(s) => self.record(&p, name, s, &mut plan)?,
            };
            records.push(record);
            plans.push(plan);
        }
        let ids = self.b.add_children(p.node, &records)?;
        for (id, plan) in ids.clone().zip(&plans) {
            let node = NodeId(id);
            if let Some(dev) = plan.new_device {
                if let Some(d) = self.b.device_mut(dev) {
                    d.mount_node = id;
                }
            }
            if plan.multi_link {
                self.stats.multi_link_nodes.push(node);
            }
            if plan.descend {
                self.queue.push_back(Pending {
                    node,
                    mnt_id: plan.mnt_id,
                    devnum: plan.devnum,
                    boundary: plan.boundary,
                    net: plan.net,
                    ino: plan.ino,
                });
            }
        }
        if self.stats.time_to_10k.is_none() && ids.end >= FIRST_NODES_MILESTONE {
            self.stats.time_to_10k = Some(self.start.elapsed());
        }
        drop(records);
        self.children = children;
        self.names = names;
        self.plans = plans;
        Ok(())
    }

    fn record<'n>(
        &mut self,
        parent: &Pending,
        name: &'n [u8],
        s: &Statx,
        plan: &mut ChildPlan,
    ) -> Result<NodeRecord<'n>, ScanError> {
        let ft = FileType::from_raw_mode(s.stx_mode.into());
        let mut flags = type_flags(ft);
        if std::str::from_utf8(name).is_err() {
            flags |= Flags::NON_UTF8;
        }
        let disk = s.stx_blocks.saturating_mul(512);
        let dn = devnum(s);
        match ft {
            FileType::Directory => self.stats.dirs += 1,
            FileType::RegularFile => {
                self.stats.files += 1;
                if disk < s.stx_size {
                    flags |= Flags::SPARSE;
                }
            }
            FileType::Symlink => self.stats.symlinks += 1,
            _ => self.stats.specials += 1,
        }
        if ft != FileType::Directory && s.stx_nlink > 1 {
            plan.multi_link = true;
            if !self.hardlinks.insert((dn, s.stx_ino)) {
                flags |= Flags::HARDLINK_DUP;
                self.stats.hardlink_dups += 1;
            }
        }

        let mnt_id = if self.has_mnt_id { s.stx_mnt_id } else { 0 };
        let mut descend = ft == FileType::Directory;
        let mut boundary = false;
        let mut net = parent.net;
        if descend {
            let new_mount = self.has_mnt_id && mnt_id != parent.mnt_id;
            if new_mount {
                flags |= Flags::MOUNT_POINT;
                self.stats.mount_points += 1;
                boundary = true;
                net = self.mounts.is_network(mnt_id);
                if self
                    .mounts
                    .is_bind_alias(mnt_id, &self.stats.root_path, self.root_mnt)
                {
                    flags |= Flags::BIND_ALIAS;
                    self.stats.bind_aliases += 1;
                    descend = false;
                }
                if net {
                    flags |= Flags::NET_FS;
                }
            } else if dn != parent.devnum {
                // Same mount, different st_dev: a btrfs subvolume (ADR-020).
                self.stats.device_boundaries += 1;
                boundary = true;
            }
        }

        let dev = match self.b.device_index(dn) {
            Some(i) => i,
            None => {
                let path = self.child_path(name);
                let i = self.b.add_device(
                    Device {
                        dev: dn,
                        mnt_id: s.stx_mnt_id,
                        rotational: self.mounts.rotational(s.stx_mnt_id),
                        mount_node: u32::MAX,
                        ..Device::default()
                    },
                    &path,
                )?;
                plan.new_device = Some(i);
                i
            }
        };
        self.running_disk = self.running_disk.saturating_add(disk);
        *plan = ChildPlan {
            descend,
            boundary,
            mnt_id,
            devnum: dn,
            net,
            ino: s.stx_ino,
            ..*plan
        };
        Ok(NodeRecord {
            name,
            flags,
            mode: s.stx_mode,
            disk_bytes: disk,
            apparent_bytes: apparent(s),
            ino: s.stx_ino,
            mtime: mtime(s),
            dev,
            uid: s.stx_uid,
            gid: s.stx_gid,
        })
    }

    fn report_progress(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_progress) >= self.opts.progress_interval {
            self.last_progress = now;
            self.progress
                .on_batch(u64::from(self.b.len()), self.running_disk);
        }
    }

    fn finish(mut self) -> Result<(Arena, ScanStats), ScanError> {
        self.b.aggregate()?;
        self.b.build_summaries();
        self.stats.nodes = u64::from(self.b.len());
        self.stats.elapsed = self.start.elapsed();
        let arena = self.b.freeze();
        self.progress.on_done(&self.stats);
        Ok((arena, self.stats))
    }
}
