// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Property test: random small trees, scanned, compared with an independent
//! model built on `std::fs::symlink_metadata`.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use gx_arena::{Arena, Flags, NodeId};
use gx_scan::{NoProgress, ScanOptions};
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Op {
    Dir(usize),
    File {
        dir: usize,
        len: usize,
        sparse_extra: u64,
    },
    Symlink {
        dir: usize,
        target: String,
    },
    HardLink {
        file: usize,
        dir: usize,
    },
    Fifo(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => any::<usize>().prop_map(Op::Dir),
        6 => (any::<usize>(), 0usize..20_000, prop_oneof![3 => Just(0u64), 1 => 1u64..1_000_000])
            .prop_map(|(dir, len, sparse_extra)| Op::File { dir, len, sparse_extra }),
        1 => (any::<usize>(), prop_oneof![Just(".".to_string()), Just("..".to_string()), Just("/".to_string()), "[a-z]{1,8}", Just("n0".to_string())])
            .prop_map(|(dir, target)| Op::Symlink { dir, target }),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(file, dir)| Op::HardLink { file, dir }),
        1 => any::<usize>().prop_map(Op::Fifo),
    ]
}

/// Creates the tree and returns its root.
fn build(root: &Path, ops: &[Op]) {
    let mut dirs: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut files: Vec<PathBuf> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let name = format!("n{i}");
        match op {
            Op::Dir(d) => {
                let p = dirs[d % dirs.len()].join(&name);
                fs::create_dir(&p).unwrap();
                dirs.push(p);
            }
            Op::File {
                dir,
                len,
                sparse_extra,
            } => {
                let p = dirs[dir % dirs.len()].join(&name);
                fs::write(&p, vec![b'x'; *len]).unwrap();
                if *sparse_extra > 0 {
                    let f = fs::OpenOptions::new().write(true).open(&p).unwrap();
                    f.set_len(*len as u64 + sparse_extra).unwrap();
                }
                files.push(p);
            }
            Op::Symlink { dir, target } => {
                let p = dirs[dir % dirs.len()].join(&name);
                std::os::unix::fs::symlink(target, &p).unwrap();
            }
            Op::HardLink { file, dir } => {
                if !files.is_empty() {
                    let p = dirs[dir % dirs.len()].join(&name);
                    fs::hard_link(&files[file % files.len()], &p).unwrap();
                }
            }
            Op::Fifo(d) => {
                let p = dirs[d % dirs.len()].join(&name);
                rustix::fs::mknodat(
                    rustix::fs::CWD,
                    &p,
                    rustix::fs::FileType::Fifo,
                    rustix::fs::Mode::from_raw_mode(0o644),
                    0,
                )
                .unwrap();
            }
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Totals {
    disk: u64,
    apparent: u64,
    nodes: u64,
    has_links: bool,
}

/// Independent model: `du`-like totals with hard links counted once.
fn model(path: &Path, seen: &mut HashSet<(u64, u64)>) -> Totals {
    let meta = fs::symlink_metadata(path).unwrap();
    let ft = meta.file_type();
    let mut t = Totals {
        nodes: 1,
        ..Totals::default()
    };
    let counts = if !ft.is_dir() && meta.nlink() > 1 {
        t.has_links = true;
        seen.insert((meta.dev(), meta.ino()))
    } else {
        true
    };
    if counts {
        t.disk = meta.blocks() * 512;
        if ft.is_file() || ft.is_symlink() {
            t.apparent = meta.size();
        }
    }
    if ft.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for e in entries {
            let c = model(&e, seen);
            t.disk += c.disk;
            t.apparent += c.apparent;
            t.nodes += c.nodes;
            t.has_links |= c.has_links;
        }
    }
    t
}

fn path_of(a: &Arena, n: NodeId) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let mut p = Vec::new();
    a.path(n, &mut p);
    PathBuf::from(std::ffi::OsStr::from_bytes(&p))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn scan_matches_independent_model(ops in proptest::collection::vec(op(), 0..60)) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("t");
        fs::create_dir(&root).unwrap();
        build(&root, &ops);

        let (arena, stats) = gx_scan::scan_with_stats(&root, &ScanOptions::default(), &NoProgress).unwrap();
        prop_assert_eq!(arena.check_invariants(), Ok(()));

        let expected = model(&root, &mut HashSet::new());
        prop_assert_eq!(arena.len() as u64, expected.nodes);
        prop_assert_eq!(stats.nodes, expected.nodes);
        prop_assert_eq!(arena.disk_bytes(arena.root()), expected.disk);
        prop_assert_eq!(arena.apparent_bytes(arena.root()), expected.apparent);

        // Every directory whose subtree has no hard links matches exactly.
        for i in 0..arena.len() as u32 {
            let n = NodeId(i);
            if !arena.flags(n).contains(Flags::DIR) {
                continue;
            }
            let t = model(&path_of(&arena, n), &mut HashSet::new());
            if t.has_links {
                continue;
            }
            prop_assert_eq!(arena.disk_bytes(n), t.disk, "disk of node {}", i);
            prop_assert_eq!(arena.apparent_bytes(n), t.apparent, "apparent of node {}", i);
        }
    }
}
