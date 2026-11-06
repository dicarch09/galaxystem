// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Scanner behaviour on special cases: non-UTF-8 names, locked folders,
//! symlink cycles, progress and capacity exhaustion.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Mutex;

use gx_arena::{Arena, Flags, NodeId};
use gx_scan::{NoProgress, Progress, ScanError, ScanOptions, ScanStats};
use gx_synth::{Kind, Params, Profile};

fn scan(root: &Path) -> (Arena, ScanStats) {
    let (a, s) = gx_scan::scan_with_stats(root, &ScanOptions::default(), &NoProgress).unwrap();
    a.check_invariants().unwrap();
    (a, s)
}

fn find(a: &Arena, name: &[u8]) -> NodeId {
    (0..a.len() as u32)
        .map(NodeId)
        .find(|&n| a.name(n) == name)
        .unwrap_or_else(|| panic!("no node named {:?}", String::from_utf8_lossy(name)))
}

#[test]
fn non_utf8_names_are_flagged_and_paths_rebuilt_byte_for_byte() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_name: &[u8] = b"\xff dir";
    let file_name: &[u8] = b"caf\xe9.txt";
    let dir = tmp.path().join(OsStr::from_bytes(dir_name));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join(OsStr::from_bytes(file_name)), b"hola").unwrap();
    fs::write(tmp.path().join("plain.txt"), b"x").unwrap();

    let (a, _) = scan(tmp.path());
    let f = find(&a, file_name);
    assert!(a.flags(f).contains(Flags::NON_UTF8));
    assert!(a.flags(find(&a, dir_name)).contains(Flags::NON_UTF8));
    assert!(!a.flags(find(&a, b"plain.txt")).contains(Flags::NON_UTF8));

    let mut path = Vec::new();
    a.path(f, &mut path);
    let mut expected = fs::canonicalize(tmp.path())
        .unwrap()
        .into_os_string()
        .into_vec();
    expected.extend_from_slice(b"/\xff dir/caf\xe9.txt");
    assert_eq!(path, expected);
    assert_eq!(fs::read(OsStr::from_bytes(&path)).unwrap(), b"hola");
}

/// Restores permissions so the temporary directory can be removed.
struct Unlock<'a>(&'a Path);
impl Drop for Unlock<'_> {
    fn drop(&mut self) {
        let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
    }
}

#[test]
fn unreadable_folder_is_locked() {
    if rustix::process::geteuid().is_root() {
        eprintln!("AVISO: prueba omitida: corre como root, que ignora los permisos");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let locked = tmp.path().join("cerrada");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("secreto"), vec![1u8; 10_000]).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let _guard = Unlock(&locked);

    let (a, stats) = scan(tmp.path());
    let n = find(&a, b"cerrada");
    assert!(a.flags(n).contains(Flags::LOCKED));
    assert!(a.children(n).is_empty());
    assert_eq!(stats.locked, 1);
    // Only its own blocks count (0 on tmpfs, one block on most others).
    let own = {
        use std::os::unix::fs::MetadataExt;
        fs::symlink_metadata(&locked).unwrap().blocks() * 512
    };
    assert_eq!(a.disk_bytes(n), own);
    assert!(!(0..a.len()).any(|i| a.name(NodeId(i as u32)) == b"secreto"));
}

fn check_pathological(files: u64) {
    let tmp = tempfile::tempdir().unwrap();
    let params = Params::new(Profile::Patologico, files, 1);
    let report = gx_synth::generate(&tmp.path().join("t"), &params, false).unwrap();
    let plan = gx_synth::plan(&params);
    assert!(
        plan.entries
            .iter()
            .any(|e| e.kind == Kind::Symlink(b"/".to_vec()))
    );

    let (a, stats) = scan(&report.root);
    // The scan terminated, indexed exactly the planned entries and never
    // went through a symlink.
    assert_eq!(a.len(), plan.entries.len());
    assert_eq!(stats.symlinks as usize, report.symlinks);
    let root = report.root.as_os_str().as_bytes();
    let mut path = Vec::new();
    for i in 0..a.len() as u32 {
        let n = NodeId(i);
        a.path(n, &mut path);
        assert!(
            path.starts_with(root) && (path.len() == root.len() || path[root.len()] == b'/'),
            "{} escapes the root",
            String::from_utf8_lossy(&path)
        );
        if a.flags(n).contains(Flags::SYMLINK) {
            assert!(a.children(n).is_empty());
            let p = a.parent(n).unwrap();
            assert!(!a.flags(p).contains(Flags::SYMLINK));
        }
    }
    let depth = (0..a.len() as u32)
        .map(|i| {
            let mut d = 0;
            let mut cur = NodeId(i);
            while let Some(p) = a.parent(cur) {
                d += 1;
                cur = p;
            }
            d
        })
        .max()
        .unwrap();
    assert!(depth >= 200, "depth {depth}");
}

#[test]
fn pathological_tree_stays_inside_root() {
    check_pathological(5_000);
}

#[test]
#[ignore = "100 000 archivos; ejecutar con --ignored"]
fn pathological_tree_stays_inside_root_100k() {
    check_pathological(100_000);
}

#[derive(Default)]
struct Recorder {
    batches: Mutex<Vec<(u64, u64)>>,
    done: Mutex<u32>,
}

impl Progress for Recorder {
    fn on_batch(&self, nodes: u64, disk: u64) {
        self.batches.lock().unwrap().push((nodes, disk));
    }
    fn on_done(&self, _: &ScanStats) {
        *self.done.lock().unwrap() += 1;
    }
}

#[test]
fn progress_and_first_nodes_milestone() {
    let tmp = tempfile::tempdir().unwrap();
    let report = gx_synth::generate(
        &tmp.path().join("t"),
        &Params::new(Profile::HomeDev, 12_000, 3),
        false,
    )
    .unwrap();
    let rec = Recorder::default();
    let opts = ScanOptions {
        progress_interval: std::time::Duration::ZERO,
        ..ScanOptions::default()
    };
    let (_, stats) = gx_scan::scan_with_stats(&report.root, &opts, &rec).unwrap();
    assert_eq!(*rec.done.lock().unwrap(), 1);
    let batches = rec.batches.lock().unwrap();
    assert!(!batches.is_empty());
    assert!(batches.windows(2).all(|w| w[0].0 <= w[1].0));
    assert!(stats.time_to_10k.is_some());
    assert!(stats.time_to_10k.unwrap() <= stats.elapsed);
}

#[test]
fn exhausted_capacity_is_an_error_not_a_panic() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..50 {
        fs::write(tmp.path().join(format!("f{i}")), b"").unwrap();
    }
    let opts = ScanOptions {
        node_capacity: 10,
        ..ScanOptions::default()
    };
    match gx_scan::scan(tmp.path(), &opts, &NoProgress) {
        Err(ScanError::Arena(_)) => {}
        other => panic!("expected an arena error, got {other:?}"),
    }
}

#[test]
fn root_must_be_a_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f");
    fs::write(&f, b"").unwrap();
    assert!(matches!(
        gx_scan::scan(&f, &ScanOptions::default(), &NoProgress),
        Err(ScanError::RootNotDirectory(_))
    ));
    assert!(matches!(
        gx_scan::scan(
            &tmp.path().join("missing"),
            &ScanOptions::default(),
            &NoProgress
        ),
        Err(ScanError::Root { .. })
    ));
}
