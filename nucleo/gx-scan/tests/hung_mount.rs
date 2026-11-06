// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A network mount that never answers `readdir` (F0-diseno.md §2.2): the
//! scan must finish, mark the mount `STALE` and index the rest of the tree.
//!
//! The mount is a FUSE filesystem served by this test, without root, with
//! subtype `sshfs` so that `/proc/self/mountinfo` reports it as a network
//! filesystem (`fuse.sshfs`). `lookup` and `getattr` answer normally;
//! `readdir` requests are held and never answered while the scan runs,
//! which leaves the reading thread blocked in the kernel. At the end they
//! are answered with `EIO` so the abandoned helper can exit, and the mount
//! is detached. Without `/dev/fuse` or `fusermount3` the test is skipped.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, INodeNo, MountOption, ReplyAttr,
    ReplyDirectory, ReplyEntry, Request,
};
use gx_arena::{Flags, NodeId};
use gx_scan::{NoProgress, ScanMode, ScanOptions};
use gx_synth::{Params, Profile};

/// Held `readdir` replies.
type Held = Arc<Mutex<Vec<ReplyDirectory>>>;

/// A filesystem with an empty root directory whose listing never ends.
struct HungFs {
    held: Held,
}

fn root_attr() -> FileAttr {
    FileAttr {
        ino: INodeNo(1),
        size: 0,
        blocks: 0,
        atime: SystemTime::UNIX_EPOCH,
        mtime: SystemTime::UNIX_EPOCH,
        ctime: SystemTime::UNIX_EPOCH,
        crtime: SystemTime::UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: rustix::process::getuid().as_raw(),
        gid: rustix::process::getgid().as_raw(),
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

impl Filesystem for HungFs {
    fn lookup(&self, _: &Request, _: INodeNo, _: &OsStr, reply: ReplyEntry) {
        reply.error(Errno::ENOENT);
    }

    fn getattr(&self, _: &Request, ino: INodeNo, _: Option<FileHandle>, reply: ReplyAttr) {
        if ino == INodeNo(1) {
            reply.attr(&Duration::from_secs(60), &root_attr());
        } else {
            reply.error(Errno::ENOENT);
        }
    }

    fn readdir(&self, _: &Request, _: INodeNo, _: FileHandle, _: u64, reply: ReplyDirectory) {
        self.held.lock().unwrap().push(reply);
    }
}

fn fuse_available() -> bool {
    Path::new("/dev/fuse").exists()
        && std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|d| d.join("fusermount3").is_file())
        })
}

#[test]
fn hung_network_mount_is_stale_and_the_rest_is_complete() {
    if !fuse_available() {
        eprintln!("AVISO: prueba omitida: no hay /dev/fuse o fusermount3");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let report = gx_synth::generate(
        &base.join("local"),
        &Params::new(Profile::HomeDev, 3_000, 5),
        false,
    )
    .unwrap();
    let mnt = base.join("remoto");
    fs::create_dir(&mnt).unwrap();

    let held: Held = Arc::default();
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName("gx-prueba".into()),
        MountOption::Subtype("sshfs".into()),
        MountOption::RO,
    ];
    let session = match fuser::spawn_mount(
        HungFs {
            held: Arc::clone(&held),
        },
        &mnt,
        &config,
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("AVISO: prueba omitida: no se pudo montar FUSE: {e}");
            return;
        }
    };

    let timeout = Duration::from_millis(300);
    let opts = ScanOptions {
        mode: ScanMode::Parallel,
        net_timeout: timeout,
        ..ScanOptions::default()
    };
    let start = Instant::now();
    let scanned = gx_scan::scan_with_stats(&base, &opts, &NoProgress);
    let elapsed = start.elapsed();

    // Release the blocked helper before checking anything, then detach.
    for reply in held.lock().unwrap().drain(..) {
        reply.error(Errno::EIO);
    }
    drop(session);

    let (a, stats) = scanned.unwrap();
    a.check_invariants().unwrap();
    assert!(
        elapsed < timeout * 4 + Duration::from_secs(2),
        "{elapsed:?}"
    );

    // The mount point: indexed, flagged and not descended.
    let m = (0..a.len() as u32)
        .map(NodeId)
        .find(|&n| a.name(n) == b"remoto")
        .unwrap();
    let flags = a.flags(m);
    for f in [Flags::MOUNT_POINT, Flags::NET_FS, Flags::STALE] {
        assert!(flags.contains(f), "{flags:?}");
    }
    assert!(a.children(m).is_empty());
    assert_eq!(stats.timed_out, 1);
    assert_eq!(stats.abandoned_threads, 1);

    // The rest of the tree is complete: the local part matches a scan of
    // it alone with the single-threaded reference.
    let (local, local_stats) = gx_scan::scan_with_stats(
        &report.root,
        &ScanOptions {
            mode: ScanMode::Sequential,
            ..ScanOptions::default()
        },
        &NoProgress,
    )
    .unwrap();
    // Besides the local tree: the scan root, the generator's manifest
    // (written next to the tree) and the mount point.
    assert_eq!(a.len(), local.len() + 3);
    assert_eq!(stats.files, local_stats.files + 1);
    let blocks = |p: &Path| {
        use std::os::unix::fs::MetadataExt;
        fs::symlink_metadata(p).unwrap().blocks() * 512
    };
    assert_eq!(
        a.disk_bytes(a.root()),
        local.disk_bytes(local.root()) + blocks(&base) + blocks(&report.manifest)
    );
}
