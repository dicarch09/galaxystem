// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Filesystem scanner (F0-diseno.md §2).
//!
//! F0.1 provides the single-threaded walk of §2.1 with the boundaries of
//! §2.4, hard links (§2.5), errors (§2.6) and progress (§2.7). Symlinks are
//! never followed: directories are opened with `openat2` and
//! `RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, entries are inspected with
//! `statx(AT_SYMLINK_NOFOLLOW)`.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![warn(missing_docs)]

pub mod fsmagic;
pub mod mountinfo;
#[allow(unsafe_code)]
mod sys;
mod walk;

use std::path::{Path, PathBuf};
use std::time::Duration;

use gx_arena::{Arena, ArenaError, DEFAULT_NODE_CAPACITY, NodeId};

pub use sys::{SubvolInfo, btrfs_subvol_info};

/// Scanner configuration.
#[derive(Clone, Debug)]
pub struct ScanOptions {
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
    walk::run(root, opts, progress)
}
