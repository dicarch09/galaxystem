// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Parallel scanner (F0-diseno.md §2.2, §2.3).
//!
//! **Pools.** Each filesystem gets its own pool of threads with
//! work-stealing deques (`crossbeam-deque`). Pools are keyed by the
//! `major:minor` that `/proc/self/mountinfo` reports for the mount, so all
//! btrfs subvolumes of one filesystem share a pool. A pool is created when
//! its first directory is found and starts with one thread; it grows up to
//! the [`ThreadPolicy`](crate::ThreadPolicy) limit while its threads have
//! queued work. A task is one directory.
//!
//! **Termination.** A global counter holds the tasks queued or running. A
//! task increments it for its subdirectories before queueing them and
//! decrements it when it ends, so it reaches zero exactly once, when the
//! whole tree is done.
//!
//! **Aggregation (§2.3).** Each directory accumulates its total directly in
//! its own size columns, which start at its own blocks. A transient counter
//! `pending[row]` holds, per directory row, the contributing subdirectories
//! still being scanned plus one for its own listing. Whoever brings it to
//! zero adds the (now final) total to the parent and decrements the
//! parent's counter, and so on upwards. Partial totals are visible while
//! the scan runs.
//!
//! **Network (§2.2).** Directories of network filesystems are read by
//! disposable helper threads that own all their data, waited for with a
//! timeout. A helper that does not answer is abandoned (it may be blocked
//! in the kernel forever) and the directory is marked `STALE`. Each mount
//! abandons at most [`ScanOptions::max_abandoned`] helpers; after that the
//! rest of it is marked `STALE` without touching it. The `statx` of a
//! network mount point seen from a local parent also runs on a helper.

use std::collections::{HashMap, HashSet};
use std::mem::MaybeUninit;
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::thread::{self, Scope, Thread};
use std::time::{Duration, Instant};

use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use crossbeam_utils::{Backoff, CachePadded};
use gx_arena::column::Column;
use gx_arena::{Arena, ArenaBuilder, ArenaError, Device, Flags, LocalCache, NodeId, NodeRecord};
use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags, RawDir, ResolveFlags, Statx, StatxFlags};
use rustix::io::Errno;

use crate::mountinfo::MountTable;
use crate::sys::SubvolInfo;
use crate::walk::{
    BTRFS_SUBVOL_INO, FIRST_NODES_MILESTONE, MASK, apparent, devnum, mtime, raise_nofile_limit,
    type_flags,
};
use crate::{Progress, ScanError, ScanErrorRecord, ScanOptions, ScanStats, fsmagic, sys};

const HARDLINK_SHARDS: usize = 64;
const S_IFDIR: u16 = 0o040000;
/// Longest sleep of an idle thread between looks for work.
const MAX_IDLE_SLEEP: Duration = Duration::from_millis(1);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Poisoning only happens if a scanner thread panicked, and then the
    // whole scan panics when the thread scope is joined.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A directory waiting to be listed.
#[derive(Clone, Copy, Debug)]
struct Task {
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

/// `statx` result of one entry.
// Kept unboxed on purpose: entries live in a reused vector, and boxing the
// common variant would add one allocation per directory entry.
#[allow(clippy::large_enum_variant)]
enum Stat {
    Done(Result<Statx, Errno>),
    /// Network mount point seen from a local parent: to be done on a helper.
    Deferred,
    /// The deferred `statx` did not answer in time.
    TimedOut,
}

struct Child {
    name: Range<usize>,
    dtype: FileType,
    stat: Stat,
}

/// What reading one directory produced, before touching the arena.
#[derive(Default)]
struct Listing {
    /// Opening failed.
    open: Option<Errno>,
    /// `fstatfs` of a boundary directory.
    magic: Option<Result<u64, Errno>>,
    /// Subvolume information of a btrfs subvolume root.
    subvol: Option<Result<SubvolInfo, Errno>>,
    names: Vec<u8>,
    children: Vec<Child>,
    /// `getdents64` failed after reading `children`.
    getdents: Option<Errno>,
}

impl Listing {
    fn clear(&mut self) {
        self.open = None;
        self.magic = None;
        self.subvol = None;
        self.names.clear();
        self.children.clear();
        self.getdents = None;
    }
}

/// Reads one directory without touching the arena: open, boundary checks,
/// `getdents64` and `statx` of every entry not selected by `defer`. Runs on
/// scanner threads and on network helpers. Returns the open directory.
fn list_dir(
    path: &[u8],
    t: &Task,
    buf: &mut [MaybeUninit<u8>],
    out: &mut Listing,
    defer: &dyn Fn(&[u8]) -> bool,
) -> Option<OwnedFd> {
    out.clear();
    let fd = match rustix::fs::openat2(
        CWD,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    ) {
        Ok(fd) => fd,
        Err(errno) => {
            out.open = Some(errno);
            return None;
        }
    };
    let mut net = t.net;
    if t.boundary {
        // `f_type` is a `long` of a different width on some targets.
        #[allow(clippy::unnecessary_cast)]
        let magic = rustix::fs::fstatfs(&fd).map(|st| fsmagic::normalize(st.f_type as i64));
        out.magic = Some(magic);
        if let Ok(m) = magic {
            if fsmagic::is_pseudo(m) {
                return Some(fd);
            }
            net |= fsmagic::is_network(m);
            if m == fsmagic::BTRFS && t.ino == BTRFS_SUBVOL_INO {
                out.subvol = Some(sys::btrfs_subvol_info(&fd));
            }
        }
    }
    {
        let mut dir = RawDir::new(&fd, buf);
        while let Some(entry) = dir.next() {
            match entry {
                Ok(e) => {
                    let name = e.file_name().to_bytes();
                    if name == b"." || name == b".." {
                        continue;
                    }
                    let start = out.names.len();
                    out.names.extend_from_slice(name);
                    out.children.push(Child {
                        name: start..out.names.len(),
                        dtype: e.file_type(),
                        stat: Stat::Deferred,
                    });
                }
                Err(errno) => {
                    out.getdents = Some(errno);
                    break;
                }
            }
        }
    }
    let mut at = AtFlags::SYMLINK_NOFOLLOW;
    if net {
        at |= AtFlags::STATX_DONT_SYNC;
    }
    for c in &mut out.children {
        let name = &out.names[c.name.clone()];
        if !defer(name) {
            c.stat = Stat::Done(rustix::fs::statx(&fd, name, at, MASK));
        }
    }
    Some(fd)
}

/// `statx` of `name` under `dir` on a disposable thread. `None` if it did
/// not answer within `timeout`; the thread is then abandoned.
fn statx_with_timeout(
    dir: &OwnedFd,
    name: &[u8],
    timeout: Duration,
) -> Option<Result<Statx, Errno>> {
    let fd = match dir.try_clone() {
        Ok(fd) => fd,
        Err(e) => return Some(Err(Errno::from_io_error(&e).unwrap_or(Errno::IO))),
    };
    let name = name.to_vec();
    let (tx, rx) = mpsc::sync_channel(1);
    let spawned = thread::Builder::new()
        .name("gx-net-statx".into())
        .spawn(move || {
            let at = AtFlags::SYMLINK_NOFOLLOW | AtFlags::STATX_DONT_SYNC;
            let _ = tx.send(rustix::fs::statx(&fd, name.as_slice(), at, MASK));
        });
    if let Err(e) = spawned {
        return Some(Err(Errno::from_io_error(&e).unwrap_or(Errno::AGAIN)));
    }
    rx.recv_timeout(timeout).ok()
}

struct NetRequest {
    path: Vec<u8>,
    task: Task,
}

/// A helper thread that lists directories of a network filesystem. It owns
/// everything it touches, so it can be abandoned while blocked.
struct NetHelper {
    tx: mpsc::Sender<NetRequest>,
    rx: mpsc::Receiver<Listing>,
}

impl NetHelper {
    fn spawn(buf_size: usize) -> std::io::Result<Self> {
        let (tx, requests) = mpsc::channel::<NetRequest>();
        let (replies, rx) = mpsc::channel();
        thread::Builder::new()
            .name("gx-net".into())
            .spawn(move || {
                let mut buf = vec![MaybeUninit::uninit(); buf_size];
                for req in requests {
                    let mut out = Listing::default();
                    drop(list_dir(&req.path, &req.task, &mut buf, &mut out, &|_| {
                        false
                    }));
                    if replies.send(out).is_err() {
                        break;
                    }
                }
            })?;
        Ok(NetHelper { tx, rx })
    }

    /// `None` if the helper did not answer in time (or died).
    fn list(&self, req: NetRequest, timeout: Duration) -> Option<Listing> {
        self.tx.send(req).ok()?;
        self.rx.recv_timeout(timeout).ok()
    }
}

type HardlinkShard = CachePadded<Mutex<HashSet<(u64, u64)>>>;

/// `(dev, ino)` pairs of multi-link files already seen (F0-diseno.md §2.5).
struct HardlinkSet {
    shards: Vec<HardlinkShard>,
}

impl HardlinkSet {
    fn new() -> Self {
        HardlinkSet {
            shards: (0..HARDLINK_SHARDS)
                .map(|_| CachePadded::new(Mutex::new(HashSet::new())))
                .collect(),
        }
    }

    /// Inserts the pair and returns whether it was new.
    fn insert(&self, dev: u64, ino: u64) -> bool {
        let h = (ino ^ dev.rotate_left(32)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let shard = (h >> (64 - HARDLINK_SHARDS.trailing_zeros())) as usize;
        lock(&self.shards[shard]).insert((dev, ino))
    }
}

/// Helper slots of a network pool.
enum Slot {
    Acquired,
    /// All slots are taken, some by live helpers of other threads.
    Busy,
    /// All slots belong to abandoned helpers.
    Exhausted,
}

/// Threads of one filesystem.
struct Pool {
    key: (u32, u32),
    net: bool,
    injector: Injector<Task>,
    stealers: Vec<Stealer<Task>>,
    /// Deques of the threads not started yet.
    spare: Mutex<Vec<Worker<Task>>>,
    started: AtomicUsize,
    /// Network pools: live plus abandoned helpers.
    slots: AtomicU32,
    /// Network pools: abandoned helpers.
    abandoned: AtomicU32,
}

impl Pool {
    fn acquire_slot(&self, max: u32) -> Slot {
        let mut cur = self.slots.load(Ordering::Acquire);
        loop {
            if cur >= max {
                return if self.abandoned.load(Ordering::Acquire) >= max {
                    Slot::Exhausted
                } else {
                    Slot::Busy
                };
            }
            match self.slots.compare_exchange_weak(
                cur,
                cur + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Slot::Acquired,
                Err(actual) => cur = actual,
            }
        }
    }

    fn release_slot(&self) {
        self.slots.fetch_sub(1, Ordering::AcqRel);
    }

    fn next_task(&self, local: &Worker<Task>) -> Option<Task> {
        local.pop().or_else(|| {
            std::iter::repeat_with(|| {
                self.injector
                    .steal_batch_and_pop(local)
                    .or_else(|| self.stealers.iter().map(Stealer::steal).collect())
            })
            .find(|s| !s.is_retry())
            .and_then(Steal::success)
        })
    }
}

/// State shared by all scanner threads.
struct Ctx<'a> {
    opts: &'a ScanOptions,
    b: ArenaBuilder,
    /// Transient aggregation counters, indexed by directory row.
    pending: Column<u32>,
    mounts: MountTable,
    /// Network mount points by absolute path, with their mount id.
    net_mounts: HashMap<Vec<u8>, u64>,
    /// Directories that contain a network mount point.
    net_mount_parents: HashSet<Vec<u8>>,
    hardlinks: HardlinkSet,
    root_path: Vec<u8>,
    root_mnt: u64,
    has_mnt_id: bool,
    start: Instant,
    tasks: CachePadded<AtomicUsize>,
    running_disk: CachePadded<AtomicU64>,
    abort: AtomicBool,
    fatal: Mutex<Option<ScanError>>,
    time_to_10k: OnceLock<Duration>,
    pools: Mutex<Vec<Arc<Pool>>>,
    stats: Mutex<Vec<ScanStats>>,
    /// Deferred mount-point `statx` calls in flight or abandoned, per mount.
    statx_slots: Mutex<HashMap<u64, u32>>,
    main: Thread,
}

impl Ctx<'_> {
    fn fail(&self, e: ScanError) {
        lock(&self.fatal).get_or_insert(e);
        self.abort.store(true, Ordering::Release);
        self.main.unpark();
    }

    fn task_done(&self) {
        if self.tasks.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.main.unpark();
        }
    }

    fn pool_key(&self, mnt_id: u64, devnum: u64) -> (u32, u32) {
        if self.has_mnt_id
            && let Some(e) = self.mounts.get(mnt_id)
        {
            return (e.major, e.minor);
        }
        (rustix::fs::major(devnum), rustix::fs::minor(devnum))
    }

    /// Mount id of the network mount point `name` under `dir`, if any.
    fn net_mount_at(&self, dir: &[u8], name: &[u8]) -> Option<u64> {
        let mut path = dir.to_vec();
        if path.last() != Some(&b'/') {
            path.push(b'/');
        }
        path.extend_from_slice(name);
        self.net_mounts.get(&path).copied()
    }

    /// Ends the own listing of directory `n` (§2.3): decrements its counter
    /// and, for every directory whose counter reaches zero, adds its final
    /// total to its parent.
    fn complete(&self, mut n: NodeId) -> Result<(), ScanError> {
        loop {
            let row = self.b.dir_index(n)?;
            // AcqRel: the last decrement sees every size added by the
            // children, which each add before decrementing.
            if self.pending.fetch_sub(row, 1).map_err(ArenaError::from)? != 1 {
                return Ok(());
            }
            if !self.b.contributes(n) {
                return Ok(());
            }
            let Some(parent) = self.b.parent(n)? else {
                return Ok(());
            };
            self.b
                .add_sizes(parent, self.b.disk_bytes(n)?, self.b.apparent_bytes(n)?)?;
            n = parent;
        }
    }

    /// Sets the counter of directory `n` to its own listing plus `subdirs`
    /// contributing subdirectories. Must happen before those are queued.
    fn set_pending(&self, n: NodeId, subdirs: u32) -> Result<(), ScanError> {
        let row = self.b.dir_index(n)?;
        self.pending
            .ensure_len(row + 1)
            .and_then(|()| self.pending.set(row, 1 + subdirs))
            .map_err(ArenaError::from)?;
        Ok(())
    }
}

/// Handle passed to every scanner thread.
struct Run<'scope, 'env> {
    ctx: &'env Ctx<'env>,
    s: &'scope Scope<'scope, 'env>,
}

impl Clone for Run<'_, '_> {
    fn clone(&self) -> Self {
        *self
    }
}
impl Copy for Run<'_, '_> {}

/// Sets the abort flag if the scanner thread panics, so that the others
/// and the main thread stop waiting for work that will never finish.
struct AbortOnPanic<'a>(&'a Ctx<'a>);

impl Drop for AbortOnPanic<'_> {
    fn drop(&mut self) {
        if thread::panicking() {
            self.0.abort.store(true, Ordering::Release);
            self.0.main.unpark();
        }
    }
}

/// Returns the pool of filesystem `key`, creating it (with one thread) if
/// it does not exist yet.
fn get_pool(
    r: Run<'_, '_>,
    key: (u32, u32),
    mnt_id: u64,
    net: bool,
) -> Result<Arc<Pool>, ScanError> {
    let ctx = r.ctx;
    let mut pools = lock(&ctx.pools);
    if let Some(p) = pools.iter().find(|p| p.key == key) {
        return Ok(Arc::clone(p));
    }
    let threads = if net {
        ctx.opts
            .threads
            .network
            .min(ctx.opts.max_abandoned as usize)
    } else {
        ctx.opts
            .threads
            .for_rotational(ctx.mounts.rotational(mnt_id))
    }
    .max(1);
    let mut workers: Vec<Worker<Task>> = (0..threads).map(|_| Worker::new_fifo()).collect();
    let stealers = workers.iter().map(Worker::stealer).collect();
    let first = workers.pop().expect("at least one thread");
    let pool = Arc::new(Pool {
        key,
        net,
        injector: Injector::new(),
        stealers,
        spare: Mutex::new(workers),
        started: AtomicUsize::new(0),
        slots: AtomicU32::new(0),
        abandoned: AtomicU32::new(0),
    });
    start_worker(r, &pool, first).map_err(ScanError::Spawn)?;
    pools.push(Arc::clone(&pool));
    Ok(pool)
}

fn start_worker(r: Run<'_, '_>, pool: &Arc<Pool>, deque: Worker<Task>) -> std::io::Result<()> {
    let i = pool.started.fetch_add(1, Ordering::Relaxed);
    let name = format!("gx-{}:{}-{i}", pool.key.0, pool.key.1);
    let p = Arc::clone(pool);
    let spawned = thread::Builder::new()
        .name(name)
        .spawn_scoped(r.s, move || worker(r, p, deque));
    if let Err(e) = spawned {
        pool.started.fetch_sub(1, Ordering::Relaxed);
        return Err(e);
    }
    Ok(())
}

/// Starts one more thread in `pool` if it is below its limit.
fn grow(r: Run<'_, '_>, pool: &Arc<Pool>) {
    if pool.started.load(Ordering::Relaxed) >= pool.stealers.len() {
        return;
    }
    let Some(deque) = lock(&pool.spare).pop() else {
        return;
    };
    // If the thread cannot be started the pool simply stays smaller.
    let _ = start_worker(r, pool, deque);
}

/// What a thread does after a task.
#[derive(PartialEq, Eq)]
enum Flow {
    /// The task is finished.
    Done,
    /// The task was put back in the pool and this thread must exit (a
    /// network pool without a free helper slot).
    Requeued,
}

fn worker(r: Run<'_, '_>, pool: Arc<Pool>, deque: Worker<Task>) {
    let ctx = r.ctx;
    let _guard = AbortOnPanic(ctx);
    let mut t = Local::new(ctx.opts);
    let backoff = Backoff::new();
    let mut sleep = Duration::from_micros(20);
    while !ctx.abort.load(Ordering::Acquire) {
        let Some(task) = pool.next_task(&deque) else {
            if ctx.tasks.load(Ordering::Acquire) == 0 {
                break;
            }
            if backoff.is_completed() {
                thread::sleep(sleep);
                sleep = (sleep * 2).min(MAX_IDLE_SLEEP);
            } else {
                backoff.snooze();
            }
            continue;
        };
        backoff.reset();
        sleep = Duration::from_micros(20);
        match t.visit(r, &pool, &deque, task) {
            Ok(Flow::Done) => ctx.task_done(),
            Ok(Flow::Requeued) => {
                while let Some(task) = deque.pop() {
                    pool.injector.push(task);
                }
                break;
            }
            Err(e) => {
                ctx.fail(e);
                break;
            }
        }
    }
    if t.helper.take().is_some() {
        pool.release_slot();
    }
    lock(&ctx.stats).push(t.stats);
}

/// Outcome of [`Local::list_remote`].
enum Remote {
    /// The listing is in `Local::listing`.
    Listed,
    /// Not listed; the directory is already marked.
    Skipped,
    /// No helper slot is free while other threads still have live helpers.
    NoSlot,
}

/// Per-child decisions taken before the ids are known.
#[derive(Clone, Copy, Default)]
struct ChildPlan {
    descend: bool,
    boundary: bool,
    new_mount: bool,
    mnt_id: u64,
    devnum: u64,
    net: bool,
    ino: u64,
    multi_link: bool,
    new_device: Option<u16>,
}

/// Per-thread state.
struct Local {
    stats: ScanStats,
    max_logged: usize,
    buf: Vec<MaybeUninit<u8>>,
    listing: Listing,
    plans: Vec<ChildPlan>,
    queued: Vec<(Task, bool)>,
    dir_path: Vec<u8>,
    cache: LocalCache,
    helper: Option<NetHelper>,
    dir_disk: u64,
}

impl Local {
    fn new(opts: &ScanOptions) -> Self {
        Local {
            stats: ScanStats::default(),
            max_logged: opts.max_logged_errors,
            buf: vec![MaybeUninit::uninit(); opts.getdents_buffer.max(4096)],
            listing: Listing::default(),
            plans: Vec::new(),
            queued: Vec::new(),
            dir_path: Vec::new(),
            cache: LocalCache::default(),
            helper: None,
            dir_disk: 0,
        }
    }

    fn log(&mut self, path: Vec<u8>, op: &'static str, errno: Errno) {
        if self.stats.error_log.len() < self.max_logged {
            self.stats.error_log.push(ScanErrorRecord {
                path,
                op,
                errno: errno.raw_os_error(),
            });
        }
    }

    fn child_path(&self, name: &[u8]) -> Vec<u8> {
        let mut p = self.dir_path.clone();
        if p.last() != Some(&b'/') {
            p.push(b'/');
        }
        p.extend_from_slice(name);
        p
    }

    /// Ends a directory whose entries are not listed.
    fn finish_empty(&self, ctx: &Ctx<'_>, node: NodeId) -> Result<Flow, ScanError> {
        ctx.set_pending(node, 0)?;
        ctx.complete(node)?;
        Ok(Flow::Done)
    }

    /// Lists a directory of a network pool on a helper, into
    /// `self.listing`.
    fn list_remote(&mut self, ctx: &Ctx<'_>, pool: &Pool, p: &Task) -> Result<Remote, ScanError> {
        if self.helper.is_none() {
            match pool.acquire_slot(ctx.opts.max_abandoned) {
                Slot::Acquired => match NetHelper::spawn(self.buf.len()) {
                    Ok(h) => self.helper = Some(h),
                    Err(e) => {
                        pool.release_slot();
                        let errno = Errno::from_io_error(&e).unwrap_or(Errno::AGAIN);
                        ctx.b.add_flags(p.node, Flags::ERROR)?;
                        self.stats.errors += 1;
                        self.log(self.dir_path.clone(), "spawn", errno);
                        return Ok(Remote::Skipped);
                    }
                },
                Slot::Busy => return Ok(Remote::NoSlot),
                Slot::Exhausted => {
                    ctx.b.add_flags(p.node, Flags::STALE)?;
                    self.stats.timed_out += 1;
                    return Ok(Remote::Skipped);
                }
            }
        }
        let helper = self.helper.as_ref().expect("helper just ensured");
        let req = NetRequest {
            path: self.dir_path.clone(),
            task: *p,
        };
        match helper.list(req, ctx.opts.net_timeout) {
            Some(listing) => {
                self.listing = listing;
                Ok(Remote::Listed)
            }
            None => {
                // Abandon it; its slot stays taken.
                self.helper = None;
                pool.abandoned.fetch_add(1, Ordering::AcqRel);
                self.stats.abandoned_threads += 1;
                self.stats.timed_out += 1;
                ctx.b.add_flags(p.node, Flags::STALE)?;
                Ok(Remote::Skipped)
            }
        }
    }

    fn visit(
        &mut self,
        r: Run<'_, '_>,
        pool: &Arc<Pool>,
        deque: &Worker<Task>,
        mut p: Task,
    ) -> Result<Flow, ScanError> {
        let ctx = r.ctx;
        let b = &ctx.b;
        b.path(p.node, &mut self.dir_path)?;

        // 1. Read the directory.
        let fd = if pool.net {
            match self.list_remote(ctx, pool, &p)? {
                Remote::Listed => None,
                Remote::Skipped => return self.finish_empty(ctx, p.node),
                Remote::NoSlot => {
                    // Give the task back to the threads that still have a
                    // helper, and exit.
                    pool.injector.push(p);
                    return Ok(Flow::Requeued);
                }
            }
        } else {
            let check = ctx.net_mount_parents.contains(&self.dir_path);
            let dir_path = &self.dir_path;
            let defer = |name: &[u8]| check && ctx.net_mount_at(dir_path, name).is_some();
            list_dir(dir_path, &p, &mut self.buf, &mut self.listing, &defer)
        };

        // 2. Opening errors.
        if let Some(errno) = self.listing.open {
            match errno {
                Errno::ACCESS => {
                    b.add_flags(p.node, Flags::LOCKED)?;
                    self.stats.locked += 1;
                }
                Errno::NOENT => {
                    // Removed between listing its parent and now.
                    b.add_flags(p.node, Flags::STALE)?;
                    self.stats.vanished += 1;
                }
                _ => {
                    b.add_flags(p.node, Flags::ERROR)?;
                    self.stats.errors += 1;
                    self.log(self.dir_path.clone(), "openat2", errno);
                }
            }
            return self.finish_empty(ctx, p.node);
        }

        // 3. Boundary checks.
        if p.boundary {
            let dev = b.dev(p.node)?;
            let magic = match self.listing.magic {
                Some(Ok(m)) => m,
                Some(Err(errno)) => {
                    self.log(self.dir_path.clone(), "fstatfs", errno);
                    0
                }
                None => 0,
            };
            b.update_device(dev, |d| {
                if d.fs_magic == 0 {
                    d.fs_magic = magic;
                }
            });
            if fsmagic::is_pseudo(magic) {
                b.add_flags(p.node, Flags::PSEUDO_FS)?;
                self.stats.pseudo_fs += 1;
                return self.finish_empty(ctx, p.node);
            }
            if fsmagic::is_network(magic) && !p.net {
                p.net = true;
                b.add_flags(p.node, Flags::NET_FS)?;
            }
            match self.listing.subvol {
                Some(Ok(info)) => {
                    let mut extra = Flags::SUBVOL;
                    if info.parent_uuid != [0; 16] {
                        extra |= Flags::SNAPSHOT_ECHO;
                    }
                    let node = p.node.0;
                    b.update_device(dev, |d| {
                        if d.mount_node == node {
                            d.generation = info.generation;
                        }
                    });
                    b.add_flags(p.node, extra)?;
                }
                Some(Err(errno)) => {
                    self.log(self.dir_path.clone(), "BTRFS_IOC_GET_SUBVOL_INFO", errno);
                    b.add_flags(p.node, Flags::SUBVOL)?;
                }
                None => {}
            }
        }
        if let Some(errno) = self.listing.getdents {
            // Keep what was read; the directory itself is marked.
            b.add_flags(p.node, Flags::ERROR)?;
            self.stats.errors += 1;
            self.log(self.dir_path.clone(), "getdents64", errno);
        }

        // 4. Deferred `statx` of network mount points, with a timeout.
        let mut listing = std::mem::take(&mut self.listing);
        if let Some(fd) = &fd {
            for c in &mut listing.children {
                if matches!(c.stat, Stat::Deferred) {
                    let name = &listing.names[c.name.clone()];
                    let mnt = ctx.net_mount_at(&self.dir_path, name).unwrap_or(0);
                    c.stat = self.deferred_statx(ctx, fd, name, mnt);
                }
            }
        }
        drop(fd);

        // 5. Build the records.
        let mut records: Vec<NodeRecord<'_>> = Vec::with_capacity(listing.children.len());
        let mut plans = std::mem::take(&mut self.plans);
        plans.clear();
        self.dir_disk = 0;
        for c in &listing.children {
            let name = &listing.names[c.name.clone()];
            let mut plan = ChildPlan::default();
            let record = match &c.stat {
                Stat::Done(Err(Errno::NOENT)) => {
                    self.stats.vanished += 1;
                    continue;
                }
                Stat::Done(Err(errno)) => {
                    let errno = *errno;
                    self.stats.errors += 1;
                    let path = self.child_path(name);
                    self.log(path, "statx", errno);
                    NodeRecord {
                        name,
                        flags: type_flags(c.dtype) | Flags::ERROR,
                        dev: b.dev(p.node)?,
                        ..NodeRecord::default()
                    }
                }
                Stat::Done(Ok(s)) => self.record(ctx, &p, name, s, &mut plan)?,
                Stat::Deferred | Stat::TimedOut => {
                    self.stats.dirs += 1;
                    self.stats.mount_points += 1;
                    self.stats.timed_out += 1;
                    NodeRecord {
                        name,
                        flags: Flags::DIR | Flags::MOUNT_POINT | Flags::NET_FS | Flags::STALE,
                        mode: S_IFDIR,
                        dev: b.dev(p.node)?,
                        ..NodeRecord::default()
                    }
                }
            };
            records.push(record);
            plans.push(plan);
        }
        let ids = b.add_children(p.node, &records, &mut self.cache)?;

        // 6. Sizes of the children that are not scanned, and the tasks.
        let (mut disk, mut app, mut subdirs) = (0u64, 0u64, 0u32);
        self.queued.clear();
        for ((id, plan), rec) in ids.clone().zip(&plans).zip(&records) {
            let node = NodeId(id);
            if let Some(dev) = plan.new_device {
                b.update_device(dev, |d| d.mount_node = id);
            }
            if plan.multi_link {
                self.stats.multi_link_nodes.push(node);
            }
            let contributes = b.contributes(node);
            if plan.descend {
                subdirs += u32::from(contributes);
                self.queued.push((
                    Task {
                        node,
                        mnt_id: plan.mnt_id,
                        devnum: plan.devnum,
                        boundary: plan.boundary,
                        net: plan.net,
                        ino: plan.ino,
                    },
                    plan.new_mount,
                ));
            } else if contributes {
                disk = disk.saturating_add(rec.disk_bytes);
                app = app.saturating_add(rec.apparent_bytes);
            }
        }
        if ids.end >= FIRST_NODES_MILESTONE {
            ctx.time_to_10k.get_or_init(|| ctx.start.elapsed());
        }
        drop(records);
        self.listing = listing;
        self.plans = plans;
        ctx.running_disk.fetch_add(self.dir_disk, Ordering::Relaxed);

        // 7. Aggregation counter and sizes first, then the tasks (§2.3).
        ctx.set_pending(p.node, subdirs)?;
        b.add_sizes(p.node, disk, app)?;
        ctx.tasks.fetch_add(self.queued.len(), Ordering::AcqRel);
        for &(task, new_mount) in &self.queued {
            let key = if new_mount {
                ctx.pool_key(task.mnt_id, task.devnum)
            } else {
                pool.key
            };
            if key == pool.key {
                deque.push(task);
            } else {
                match get_pool(r, key, task.mnt_id, task.net) {
                    Ok(other) => other.injector.push(task),
                    Err(e) => return Err(e),
                }
            }
        }
        if deque.len() > 1 {
            grow(r, pool);
        }
        ctx.complete(p.node)?;
        Ok(Flow::Done)
    }

    fn deferred_statx(&mut self, ctx: &Ctx<'_>, fd: &OwnedFd, name: &[u8], mnt: u64) -> Stat {
        let max = ctx.opts.max_abandoned;
        {
            let mut slots = lock(&ctx.statx_slots);
            let used = slots.entry(mnt).or_insert(0);
            if *used >= max {
                return Stat::TimedOut;
            }
            *used += 1;
        }
        match statx_with_timeout(fd, name, ctx.opts.net_timeout) {
            Some(r) => {
                if let Some(used) = lock(&ctx.statx_slots).get_mut(&mnt) {
                    *used -= 1;
                }
                Stat::Done(r)
            }
            None => {
                self.stats.abandoned_threads += 1;
                Stat::TimedOut
            }
        }
    }

    fn record<'n>(
        &mut self,
        ctx: &Ctx<'_>,
        parent: &Task,
        name: &'n [u8],
        s: &Statx,
        plan: &mut ChildPlan,
    ) -> Result<NodeRecord<'n>, ScanError> {
        let b = &ctx.b;
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
            if !ctx.hardlinks.insert(dn, s.stx_ino) {
                flags |= Flags::HARDLINK_DUP;
                self.stats.hardlink_dups += 1;
            }
        }

        let mnt_id = if ctx.has_mnt_id { s.stx_mnt_id } else { 0 };
        let mut descend = ft == FileType::Directory;
        let mut boundary = false;
        let mut new_mount = false;
        let mut net = parent.net;
        if descend {
            new_mount = ctx.has_mnt_id && mnt_id != parent.mnt_id;
            if new_mount {
                flags |= Flags::MOUNT_POINT;
                self.stats.mount_points += 1;
                boundary = true;
                net = ctx.mounts.is_network(mnt_id);
                if ctx
                    .mounts
                    .is_bind_alias(mnt_id, &ctx.root_path, ctx.root_mnt)
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

        let dev = match b.device_index(dn) {
            Some(i) => i,
            None => {
                let path = self.child_path(name);
                let (i, new) = b.add_device(
                    Device {
                        dev: dn,
                        mnt_id: s.stx_mnt_id,
                        rotational: ctx.mounts.rotational(s.stx_mnt_id),
                        mount_node: u32::MAX,
                        ..Device::default()
                    },
                    &path,
                )?;
                if new {
                    plan.new_device = Some(i);
                }
                i
            }
        };
        self.dir_disk = self.dir_disk.saturating_add(disk);
        *plan = ChildPlan {
            descend,
            boundary,
            new_mount,
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
}

fn merge(total: &mut ScanStats, part: ScanStats, max_logged: usize) {
    total.dirs += part.dirs;
    total.files += part.files;
    total.symlinks += part.symlinks;
    total.specials += part.specials;
    total.locked += part.locked;
    total.errors += part.errors;
    total.vanished += part.vanished;
    total.hardlink_dups += part.hardlink_dups;
    total.mount_points += part.mount_points;
    total.device_boundaries += part.device_boundaries;
    total.pseudo_fs += part.pseudo_fs;
    total.bind_aliases += part.bind_aliases;
    total.timed_out += part.timed_out;
    total.abandoned_threads += part.abandoned_threads;
    let room = max_logged.saturating_sub(total.error_log.len());
    total
        .error_log
        .extend(part.error_log.into_iter().take(room));
    total.multi_link_nodes.extend(part.multi_link_nodes);
}

/// Network mount points of the table, and the directories containing them.
fn network_mounts(mounts: &MountTable) -> (HashMap<Vec<u8>, u64>, HashSet<Vec<u8>>) {
    let mut points = HashMap::new();
    let mut parents = HashSet::new();
    for e in mounts.entries() {
        if !mounts.is_network(e.id) {
            continue;
        }
        points.insert(e.mount_point.clone(), e.id);
        let mp = &e.mount_point;
        let parent = match mp.iter().rposition(|&c| c == b'/') {
            Some(0) => b"/".to_vec(),
            Some(i) => mp[..i].to_vec(),
            None => continue,
        };
        parents.insert(parent);
    }
    (points, parents)
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
    let root_path = canon.as_os_str().as_bytes().to_vec();
    let has_mnt_id = StatxFlags::from_bits_retain(stx.stx_mask).contains(StatxFlags::MNT_ID);
    let mounts = MountTable::load();

    let mut b = ArenaBuilder::new(opts.node_capacity)?;
    let dn = devnum(&stx);
    let (dev, _) = b.add_device(
        Device {
            dev: dn,
            mnt_id: stx.stx_mnt_id,
            rotational: mounts.rotational(stx.stx_mnt_id),
            mount_node: 0,
            ..Device::default()
        },
        &root_path,
    )?;
    let net = has_mnt_id && mounts.is_network(stx.stx_mnt_id);
    let mut flags = Flags::DIR;
    if net {
        flags |= Flags::NET_FS;
    }
    if std::str::from_utf8(&root_path).is_err() {
        flags |= Flags::NON_UTF8;
    }
    let root_node = b.set_root(
        &root_path,
        &NodeRecord {
            name: b"",
            flags,
            mode: stx.stx_mode,
            disk_bytes: stx.stx_blocks * 512,
            apparent_bytes: apparent(&stx),
            ino: stx.stx_ino,
            mtime: mtime(&stx),
            dev,
            uid: stx.stx_uid,
            gid: stx.stx_gid,
        },
    )?;
    let pending = Column::with_capacity(b.node_capacity()).map_err(ArenaError::from)?;
    let (net_mounts, net_mount_parents) = network_mounts(&mounts);
    let mut stats = ScanStats {
        root_path: root_path.clone(),
        node_capacity: b.node_capacity(),
        dirs: 1,
        ..ScanStats::default()
    };
    let ctx = Ctx {
        opts,
        b,
        pending,
        mounts,
        net_mounts,
        net_mount_parents,
        hardlinks: HardlinkSet::new(),
        root_path,
        root_mnt: stx.stx_mnt_id,
        has_mnt_id,
        start,
        tasks: CachePadded::new(AtomicUsize::new(1)),
        running_disk: CachePadded::new(AtomicU64::new(0)),
        abort: AtomicBool::new(false),
        fatal: Mutex::new(None),
        time_to_10k: OnceLock::new(),
        pools: Mutex::new(Vec::new()),
        stats: Mutex::new(Vec::new()),
        statx_slots: Mutex::new(HashMap::new()),
        main: thread::current(),
    };
    let root_task = Task {
        node: root_node,
        mnt_id: stx.stx_mnt_id,
        devnum: dn,
        boundary: true,
        net,
        ino: stx.stx_ino,
    };

    thread::scope(|s| {
        let r = Run { ctx: &ctx, s };
        let key = ctx.pool_key(root_task.mnt_id, root_task.devnum);
        match get_pool(r, key, root_task.mnt_id, net) {
            Ok(pool) => pool.injector.push(root_task),
            Err(e) => {
                ctx.fail(e);
                return;
            }
        }
        let interval = opts.progress_interval.max(Duration::from_millis(1));
        let mut last = Instant::now();
        loop {
            thread::park_timeout(interval);
            if ctx.tasks.load(Ordering::Acquire) == 0 || ctx.abort.load(Ordering::Acquire) {
                break;
            }
            let now = Instant::now();
            if now.duration_since(last) >= interval {
                last = now;
                progress.on_batch(
                    u64::from(ctx.b.len()),
                    ctx.running_disk.load(Ordering::Relaxed),
                );
            }
        }
    });

    let Ctx {
        mut b,
        fatal,
        time_to_10k,
        pools,
        stats: parts,
        ..
    } = ctx;
    if let Some(e) = fatal.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(e);
    }
    for part in parts.into_inner().unwrap_or_else(PoisonError::into_inner) {
        merge(&mut stats, part, opts.max_logged_errors);
    }
    stats.threads = pools
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|p| p.started.load(Ordering::Relaxed) as u64)
        .sum();
    stats.time_to_10k = time_to_10k.into_inner();
    b.build_summaries();
    stats.nodes = u64::from(b.len());
    stats.elapsed = start.elapsed();
    let arena = b.freeze();
    progress.on_done(&stats);
    Ok((arena, stats))
}
