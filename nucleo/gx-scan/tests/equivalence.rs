// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The parallel scanner against the single-threaded reference (F0.2).
//! Ids depend on the traversal order, so nodes are compared by path.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use gx_arena::{Arena, Flags, NodeId};
use gx_scan::{NoProgress, ScanMode, ScanOptions, ScanStats, ThreadPolicy};
use gx_synth::{Params, Profile};

const PROFILES: [Profile; 4] = [
    Profile::HomeDev,
    Profile::Fotografo,
    Profile::Servidor,
    Profile::Patologico,
];

fn scan(root: &Path, mode: ScanMode, threads: usize) -> (Arena, ScanStats) {
    let opts = ScanOptions {
        mode,
        threads: ThreadPolicy::uniform(threads),
        ..ScanOptions::default()
    };
    let (a, s) = gx_scan::scan_with_stats(root, &opts, &NoProgress).unwrap();
    a.check_invariants().unwrap();
    (a, s)
}

/// Per path: disk, apparent, flags without `HARDLINK_DUP`, child count.
type Entry = (u64, u64, u16, u32);

fn by_path(a: &Arena) -> HashMap<Vec<u8>, Entry> {
    let mut out = HashMap::with_capacity(a.len());
    let mut path = Vec::new();
    for i in 0..a.len() as u32 {
        let n = NodeId(i);
        a.path(n, &mut path);
        let flags = a.flags(n).bits() & !Flags::HARDLINK_DUP.bits();
        let entry = (
            a.disk_bytes(n),
            a.apparent_bytes(n),
            flags,
            a.children(n).len() as u32,
        );
        assert!(out.insert(path.clone(), entry).is_none(), "duplicate path");
    }
    out
}

/// Paths of the directories with a multi-link file below them: which link
/// counts depends on the traversal order (F0-diseno.md §2.5).
fn hardlink_ancestors(a: &Arena, s: &ScanStats) -> HashSet<Vec<u8>> {
    let mut out = HashSet::new();
    let mut path = Vec::new();
    for &n in &s.multi_link_nodes {
        let mut cur = a.parent(n);
        while let Some(p) = cur {
            a.path(p, &mut path);
            out.insert(path.clone());
            cur = a.parent(p);
        }
    }
    out
}

fn assert_equivalent(root: &Path, threads: usize) -> (ScanStats, ScanStats) {
    let (sa, ss) = scan(root, ScanMode::Sequential, 1);
    let (pa, ps) = scan(root, ScanMode::Parallel, threads);
    compare(&sa, &ss, &pa, &ps);
    (ss, ps)
}

fn compare(sa: &Arena, ss: &ScanStats, pa: &Arena, ps: &ScanStats) {
    assert_eq!(sa.len(), pa.len(), "node count");
    let (sr, pr) = (sa.root(), pa.root());
    assert_eq!(sa.disk_bytes(sr), pa.disk_bytes(pr), "root disk total");
    assert_eq!(
        sa.apparent_bytes(sr),
        pa.apparent_bytes(pr),
        "root apparent total"
    );
    for (name, a, b) in [
        ("dirs", ss.dirs, ps.dirs),
        ("files", ss.files, ps.files),
        ("symlinks", ss.symlinks, ps.symlinks),
        ("specials", ss.specials, ps.specials),
        ("hardlink_dups", ss.hardlink_dups, ps.hardlink_dups),
        ("errors", ss.errors, ps.errors),
        ("locked", ss.locked, ps.locked),
        ("mount_points", ss.mount_points, ps.mount_points),
    ] {
        assert_eq!(a, b, "stats.{name}");
    }

    let seq = by_path(sa);
    let par = by_path(pa);
    let mut skip = hardlink_ancestors(sa, ss);
    skip.extend(hardlink_ancestors(pa, ps));
    for (path, s) in &seq {
        let p = par
            .get(path)
            .unwrap_or_else(|| panic!("{} missing", String::from_utf8_lossy(path)));
        let show = String::from_utf8_lossy(path);
        assert_eq!(s.2, p.2, "flags of {show}");
        assert_eq!(s.3, p.3, "children of {show}");
        if !skip.contains(path) {
            assert_eq!((s.0, s.1), (p.0, p.1), "sizes of {show}");
        }
    }
}

#[test]
fn parallel_matches_sequential_in_every_profile() {
    for profile in PROFILES {
        let tmp = tempfile::tempdir().unwrap();
        let params = Params::new(profile, 20_000, 1);
        let report = gx_synth::generate(&tmp.path().join("t"), &params, false).unwrap();
        for threads in [1, 3, 16] {
            let (ss, ps) = assert_equivalent(&report.root, threads);
            println!(
                "{profile:?} threads={threads}: {} nodes, seq {:?}, par {:?} ({} threads)",
                ss.nodes, ss.elapsed, ps.elapsed, ps.threads
            );
        }
    }
}

#[test]
#[ignore = "50 escaneos de 100 000 archivos; ejecutar con --ignored"]
fn stress_pathological_fifty_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let params = Params::new(Profile::Patologico, 100_000, 1);
    let report = gx_synth::generate(&tmp.path().join("t"), &params, false).unwrap();
    let (sa, ss) = scan(&report.root, ScanMode::Sequential, 1);
    let threads = ThreadPolicy::default().ssd;
    for run in 0..50 {
        let (pa, ps) = scan(&report.root, ScanMode::Parallel, threads);
        compare(&sa, &ss, &pa, &ps);
        if run % 10 == 0 {
            println!("run {run}: {:?} with {} threads", ps.elapsed, ps.threads);
        }
    }
}
