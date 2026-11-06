// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Filesystem scanner (F0-diseno.md §2).
//!
//! Two implementations share the boundaries of §2.4, hard links (§2.5),
//! errors (§2.6) and progress (§2.7): the single-threaded walk of §2.1
//! ([`ScanMode::Sequential`], the reference for equivalence tests) and the
//! parallel scanner of §2.2–§2.3 ([`ScanMode::Parallel`]). Symlinks are
//! never followed: directories are opened with `openat2` and
//! `RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, entries are inspected with
//! `statx(AT_SYMLINK_NOFOLLOW)`.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![warn(missing_docs)]

pub mod fsmagic;
pub mod mountinfo;
mod par;
#[allow(unsafe_code)]
mod sys;
mod walk;

use std::path::{Path, PathBuf};
use std::time::Duration;

use gx_arena::{Arena, ArenaError, DEFAULT_NODE_CAPACITY, NodeId};

pub use sys::{SubvolInfo, btrfs_subvol_info};

/// Scanner implementation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanMode {
    /// One thread, breadth-first (F0.1). Reference for equivalence tests.
    Sequential,
    /// One work-stealing thread pool per filesystem (F0-diseno.md §2.2).
    #[default]
    Parallel,
}

/// How the parallel scanner obtains the metadata of directory entries (the
/// A/B of F0-diseno.md §2.2). `getdents64` is synchronous in both cases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatxBackend {
    /// One synchronous `statx` per entry.
    #[default]
    Sync,
    /// Batches of `IORING_OP_STATX` per directory, one ring per thread.
    /// Needs the `uring` feature; without it, or when io_uring is
    /// unavailable, the scan falls back to [`StatxBackend::Sync`] and
    /// reports it in [`ScanStats::uring_error`].
    Uring,
}

/// Threads per filesystem pool in [`ScanMode::Parallel`] (F0-diseno.md
/// §2.2). Pools start with one thread and grow up to these limits while
/// there is queued work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadPolicy {
    /// Non-rotational disks (`rotational = 0`).
    pub ssd: usize,
    /// Rotational disks (`rotational = 1`).
    pub hdd: usize,
    /// Unknown kind (tmpfs, loop devices, …).
    pub unknown: usize,
    /// Network filesystems. Capped by [`ScanOptions::max_abandoned`]: each
    /// thread abandons at most one blocked helper.
    pub network: usize,
}

impl ThreadPolicy {
    /// The same number of threads for every kind of filesystem.
    pub fn uniform(threads: usize) -> Self {
        ThreadPolicy {
            ssd: threads,
            hdd: threads,
            unknown: threads,
            network: threads,
        }
    }

    /// Threads for a local filesystem with the given rotational flag
    /// (0, 1, or 2 if unknown).
    pub fn for_rotational(&self, rotational: u8) -> usize {
        match rotational {
            0 => self.ssd,
            1 => self.hdd,
            _ => self.unknown,
        }
    }
}

impl Default for ThreadPolicy {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        ThreadPolicy {
            ssd: (cores * 2).min(32),
            hdd: 2,
            unknown: cores,
            network: 4,
        }
    }
}

/// Scanner configuration.
#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// Implementation to use.
    pub mode: ScanMode,
    /// Threads per pool ([`ScanMode::Parallel`] only).
    pub threads: ThreadPolicy,
    /// `statx` backend ([`ScanMode::Parallel`] only; network directories
    /// always use synchronous `statx` on their helpers).
    pub statx: StatxBackend,
    /// Maximum time to read one directory of a network filesystem before
    /// marking it `STALE` ([`ScanMode::Parallel`] only).
    pub net_timeout: Duration,
    /// Maximum helper threads abandoned per network mount
    /// ([`ScanMode::Parallel`] only). Once reached, the rest of the mount is
    /// marked `STALE` without touching it.
    pub max_abandoned: u32,
    /// Requested node capacity (the arena may fall back to less).
    pub node_capacity: u32,
    /// Size of the `getdents64` buffer.
    pub getdents_buffer: usize,
    /// Minimum interval between [`Progress::on_batch`] calls.
    pub progress_interval: Duration,
    /// Maximum number of entries kept in [`ScanStats::error_log`].
    pub max_logged_errors: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            mode: ScanMode::default(),
            threads: ThreadPolicy::default(),
            statx: StatxBackend::default(),
            net_timeout: Duration::from_secs(5),
            max_abandoned: 4,
            node_capacity: DEFAULT_NODE_CAPACITY,
            getdents_buffer: 256 * 1024,
            progress_interval: Duration::from_millis(50),
            max_logged_errors: 10_000,
        }
    }
}

/// Progress callbacks (F0-diseno.md §2.7).
pub trait Progress: Sync {
    /// Called at most once per [`ScanOptions::progress_interval`].
    fn on_batch(&self, nodes: u64, disk_bytes: u64);
    /// Called once when the scan finishes.
    fn on_done(&self, stats: &ScanStats);
}

/// A [`Progress`] that ignores everything.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProgress;

impl Progress for NoProgress {
    fn on_batch(&self, _: u64, _: u64) {}
    fn on_done(&self, _: &ScanStats) {}
}

/// One logged error (F0-diseno.md §2.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanErrorRecord {
    /// Path of the entry.
    pub path: Vec<u8>,
    /// Operation that failed (`"openat2"`, `"statx"`, `"getdents64"`, …).
    pub op: &'static str,
    /// `errno`.
    pub errno: i32,
}

/// Scan statistics.
#[derive(Clone, Debug, Default)]
pub struct ScanStats {
    /// Canonical scanned root.
    pub root_path: Vec<u8>,
    /// Nodes indexed.
    pub nodes: u64,
    /// Directories.
    pub dirs: u64,
    /// Regular files.
    pub files: u64,
    /// Symlinks.
    pub symlinks: u64,
    /// Devices, sockets and fifos.
    pub specials: u64,
    /// Directories marked `LOCKED`.
    pub locked: u64,
    /// Entries marked `ERROR`.
    pub errors: u64,
    /// Entries that disappeared during the scan (`ENOENT`), not indexed.
    pub vanished: u64,
    /// Nodes marked `HARDLINK_DUP`.
    pub hardlink_dups: u64,
    /// Nodes marked `MOUNT_POINT`.
    pub mount_points: u64,
    /// Directories on a different device than their parent without a mount
    /// (btrfs subvolumes).
    pub device_boundaries: u64,
    /// Nodes marked `PSEUDO_FS`.
    pub pseudo_fs: u64,
    /// Nodes marked `BIND_ALIAS`.
    pub bind_aliases: u64,
    /// Directories of network filesystems marked `STALE` because they did
    /// not answer in time (or their mount had too many abandoned threads).
    pub timed_out: u64,
    /// Helper threads abandoned while blocked on a network filesystem.
    pub abandoned_threads: u64,
    /// Scanner threads started (1 in [`ScanMode::Sequential`]).
    pub threads: u64,
    /// Threads that used an io_uring ring ([`StatxBackend::Uring`]).
    pub uring_threads: u64,
    /// `errno` of the first failure to create a ring, if any thread fell
    /// back to synchronous `statx`.
    pub uring_error: Option<i32>,
    /// Total time.
    pub elapsed: Duration,
    /// Time until 10 000 nodes were indexed, if reached.
    pub time_to_10k: Option<Duration>,
    /// Node capacity obtained for the arena.
    pub node_capacity: u32,
    /// First errors, with their errno.
    pub error_log: Vec<ScanErrorRecord>,
    /// Non-directory nodes with `nlink > 1` (transient; not stored in the
    /// arena). Used to exclude subtrees from per-folder comparisons.
    pub multi_link_nodes: Vec<NodeId>,
}

/// Fatal scan errors. Errors of individual entries never abort the scan.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The root cannot be resolved or inspected.
    #[error("cannot access scan root {}: {source}", path.display())]
    Root {
        /// Requested root.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The root is not a directory.
    #[error("scan root {} is not a directory", .0.display())]
    RootNotDirectory(PathBuf),
    /// The arena could not be reserved or ran out of capacity.
    #[error("arena: {0}")]
    Arena(#[from] ArenaError),
    /// No scanner thread could be started.
    #[error("cannot start scanner threads: {0}")]
    Spawn(std::io::Error),
}

/// Scans `root` and returns the arena (F0-diseno.md §8).
pub fn scan(root: &Path, opts: &ScanOptions, progress: &dyn Progress) -> Result<Arena, ScanError> {
    scan_with_stats(root, opts, progress).map(|(arena, _)| arena)
}

/// Like [`scan`], also returning the statistics.
pub fn scan_with_stats(
    root: &Path,
    opts: &ScanOptions,
    progress: &dyn Progress,
) -> Result<(Arena, ScanStats), ScanError> {
    match opts.mode {
        ScanMode::Sequential => walk::run(root, opts, progress),
        ScanMode::Parallel => par::run(root, opts, progress),
    }
}
