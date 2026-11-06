// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Node arena: core types and columnar storage (F0-diseno.md §1).
//!
//! The arena stores one node per filesystem entry in a structure of arrays.
//! [`ArenaBuilder`] is the writable form used while scanning; [`Arena`] is the
//! frozen, read-only form with the API of F0-diseno.md §8.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![warn(missing_docs)]

mod arena;
mod builder;
mod class;
#[allow(unsafe_code)]
pub mod column;
mod flags;
mod tables;

pub use arena::{Arena, InvariantError, MemoryUsage};
pub use builder::{ArenaBuilder, ArenaError, DEFAULT_NODE_CAPACITY, MIN_NODE_CAPACITY, NodeRecord};
pub use class::{Class, MAX_EXTENSION_LEN, extension};
pub use flags::Flags;
pub use tables::{Device, DirRow, OWNER_OVERFLOW, SUMMARY_MIN_CHILDREN, Summary};

/// Identifier of a node: its index in every arena column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    /// Sentinel meaning "no node"; the parent of the root (invariant I1).
    pub const NONE: NodeId = NodeId(u32::MAX);
}

/// Builds the path of `n` by following `parent` up to the root.
fn path_from_chain(
    n: u32,
    parent: impl Fn(u32) -> u32,
    mut push_name: impl FnMut(u32, &mut Vec<u8>),
    root: &[u8],
    out: &mut Vec<u8>,
) {
    out.clear();
    let mut chain = Vec::new();
    let mut cur = n;
    while cur != 0 && cur != NodeId::NONE.0 {
        chain.push(cur);
        cur = parent(cur);
    }
    out.extend_from_slice(root);
    for &id in chain.iter().rev() {
        if out.last() != Some(&b'/') {
            out.push(b'/');
        }
        push_name(id, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &[u8], flags: Flags, disk: u64) -> NodeRecord<'_> {
        NodeRecord {
            name,
            flags,
            mode: if flags.contains(Flags::DIR) {
                0o040755
            } else {
                0o100644
            },
            disk_bytes: disk,
            apparent_bytes: disk / 2,
            ..NodeRecord::default()
        }
    }

    fn sample() -> Arena {
        let mut b = ArenaBuilder::new(1 << 12).unwrap();
        let root = b.set_root(b"/data", &rec(b"", Flags::DIR, 4096)).unwrap();
        let kids = b
            .add_children(
                root,
                &[
                    rec(b"docs", Flags::DIR, 4096),
                    rec(b"a.PNG", Flags::EMPTY, 1000),
                    rec(b"dup", Flags::HARDLINK_DUP, 1000),
                ],
            )
            .unwrap();
        assert_eq!(kids, 1..4);
        b.add_children(NodeId(1), &[rec(b"x.txt", Flags::EMPTY, 512)])
            .unwrap();
        b.add_children(NodeId(1), &[]).unwrap_err();
        b.aggregate().unwrap();
        b.build_summaries();
        b.freeze()
    }

    #[test]
    fn build_and_read() {
        let a = sample();
        a.check_invariants().unwrap();
        assert_eq!(a.len(), 5);
        assert_eq!(a.name(a.root()), b"/data");
        assert_eq!(a.children(a.root()), 1..4);
        assert_eq!(a.children(NodeId(2)), 0..0);
        assert_eq!(a.parent(NodeId(4)), Some(NodeId(1)));
        assert_eq!(a.parent(a.root()), None);
        assert_eq!(a.class(NodeId(2)), Class::Image);
        assert_eq!(a.extension(NodeId(2)), b"png");
        // docs = 4096 + 512; root = 4096 + docs + a.PNG (dup does not count).
        assert_eq!(a.disk_bytes(NodeId(1)), 4608);
        assert_eq!(a.disk_bytes(a.root()), 4096 + 4608 + 1000);
        assert_eq!(a.disk_bytes(NodeId(3)), 1000);
        let mut p = Vec::new();
        a.path(NodeId(4), &mut p);
        assert_eq!(p, b"/data/docs/x.txt");
        a.path(a.root(), &mut p);
        assert_eq!(p, b"/data");
        assert_eq!(a.memory_usage().hot, 5 * 24);
    }

    #[test]
    fn root_slash_path_has_no_double_slash() {
        let mut b = ArenaBuilder::new(16).unwrap();
        let root = b.set_root(b"/", &rec(b"", Flags::DIR, 0)).unwrap();
        b.add_children(root, &[rec(b"etc", Flags::DIR, 0)]).unwrap();
        let a = b.freeze();
        let mut p = Vec::new();
        a.path(NodeId(1), &mut p);
        assert_eq!(p, b"/etc");
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        let mut b = ArenaBuilder::new(16).unwrap();
        assert!(b.set_root(b"relative", &rec(b"", Flags::DIR, 0)).is_err());
        assert!(b.set_root(b"/f", &rec(b"", Flags::EMPTY, 0)).is_err());
        let root = b.set_root(b"/r", &rec(b"", Flags::DIR, 0)).unwrap();
        assert!(b.set_root(b"/r", &rec(b"", Flags::DIR, 0)).is_err());
        for bad in [&b""[..], b"a/b", b"a\0b", &[b'x'; 256]] {
            assert!(matches!(
                b.add_children(root, &[rec(bad, Flags::EMPTY, 0)]),
                Err(ArenaError::InvalidName { .. })
            ));
        }
        b.add_children(root, &[rec(b"f", Flags::EMPTY, 0)]).unwrap();
        assert_eq!(
            b.add_children(NodeId(1), &[]),
            Err(ArenaError::NotADirectory(1))
        );
        assert_eq!(
            b.add_children(NodeId(9), &[]),
            Err(ArenaError::UnknownNode(9))
        );
    }

    #[test]
    fn capacity_exhaustion_is_an_error() {
        let mut b = ArenaBuilder::new(3).unwrap();
        let root = b.set_root(b"/r", &rec(b"", Flags::DIR, 0)).unwrap();
        let kids = [
            rec(b"a", Flags::EMPTY, 0),
            rec(b"b", Flags::EMPTY, 0),
            rec(b"c", Flags::EMPTY, 0),
        ];
        assert!(matches!(
            b.add_children(root, &kids),
            Err(ArenaError::Column(_))
        ));
        assert_eq!(b.len(), 1);
        b.add_children(root, &kids[..2]).unwrap();
        let a = b.freeze();
        a.check_invariants().unwrap();
    }

    #[test]
    fn device_change_does_not_contribute() {
        let mut b = ArenaBuilder::new(16).unwrap();
        let root = b.set_root(b"/r", &rec(b"", Flags::DIR, 100)).unwrap();
        let mut sub = rec(b"subvol", Flags::DIR | Flags::SUBVOL, 100);
        sub.dev = 1;
        b.add_children(root, &[sub]).unwrap();
        let mut f = rec(b"f", Flags::EMPTY, 50);
        f.dev = 1;
        b.add_children(NodeId(1), &[f]).unwrap();
        b.aggregate().unwrap();
        let a = b.freeze();
        a.check_invariants().unwrap();
        assert_eq!(a.disk_bytes(NodeId(1)), 150);
        assert_eq!(a.disk_bytes(a.root()), 100);
    }

    #[test]
    fn invariant_violations_are_detected() {
        // Corrupt a builder through its columns and check the error.
        let mut b = ArenaBuilder::new(16).unwrap();
        let root = b.set_root(b"/r", &rec(b"", Flags::DIR, 0)).unwrap();
        b.add_children(
            root,
            &[rec(b"d", Flags::DIR, 10), rec(b"f", Flags::EMPTY, 5)],
        )
        .unwrap();
        b.aggregate().unwrap();
        let a = b.freeze();
        a.check_invariants().unwrap();
        // Root total of 15 is consistent; lowering it must fail I4.
        let mut b = ArenaBuilder::new(16).unwrap();
        let root = b.set_root(b"/r", &rec(b"", Flags::DIR, 0)).unwrap();
        b.add_children(root, &[rec(b"f", Flags::EMPTY, 5)]).unwrap();
        // No aggregation: root stays at 0 < 5.
        let a = b.freeze();
        assert!(matches!(
            a.check_invariants(),
            Err(InvariantError::Total { node: 0, .. })
        ));
    }

    #[test]
    fn summaries_for_large_directories() {
        let mut b = ArenaBuilder::new(4096).unwrap();
        let root = b.set_root(b"/r", &rec(b"", Flags::DIR, 0)).unwrap();
        let names: Vec<Vec<u8>> = (0..1100)
            .map(|i| format!("f{i}.jpg").into_bytes())
            .collect();
        let mut kids: Vec<NodeRecord<'_>> =
            names.iter().map(|n| rec(n, Flags::EMPTY, 10)).collect();
        kids.push(rec(b"sub", Flags::DIR, 0));
        b.add_children(root, &kids).unwrap();
        b.aggregate().unwrap();
        b.build_summaries();
        let a = b.freeze();
        let s = a.summary(a.root()).unwrap();
        assert_eq!(s.count[Class::Image as usize], 1100);
        assert_eq!(s.bytes[Class::Image as usize], 11000);
        assert_eq!(s.count.iter().sum::<u32>(), 1100);
        assert_eq!(a.summary(NodeId(1)), None);
    }
}
