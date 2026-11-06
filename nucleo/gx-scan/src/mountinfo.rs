// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Parser for `/proc/self/mountinfo`, used to recognise network filesystems
//! and bind mounts (F0-diseno.md §2.4).

use std::collections::HashMap;

/// One line of `mountinfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountEntry {
    /// Mount id (matches `stx_mnt_id`).
    pub id: u64,
    /// Parent mount id.
    pub parent: u64,
    /// Device major.
    pub major: u32,
    /// Device minor.
    pub minor: u32,
    /// Root of the mount inside its filesystem.
    pub root: Vec<u8>,
    /// Mount point.
    pub mount_point: Vec<u8>,
    /// Filesystem type.
    pub fstype: Vec<u8>,
    /// Mount source.
    pub source: Vec<u8>,
}

/// Network filesystem types, besides those detected by `statfs` magic.
const NET_TYPES: &[&[u8]] = &[
    b"nfs",
    b"nfs4",
    b"cifs",
    b"smb3",
    b"smbfs",
    b"ncpfs",
    b"afs",
    b"ceph",
    b"glusterfs",
    b"9p",
    b"davfs",
    b"fuse.sshfs",
    b"fuse.rclone",
    b"fuse.s3fs",
    b"fuse.gcsfuse",
    b"fuse.davfs2",
    b"fuse.glusterfs",
    b"fuse.cephfs",
];

/// Parsed mount table.
#[derive(Clone, Debug, Default)]
pub struct MountTable {
    entries: Vec<MountEntry>,
    by_id: HashMap<u64, usize>,
}

/// Decodes the octal escapes (`\040` …) used by mountinfo.
fn unescape(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut i = 0;
    while i < field.len() {
        if field[i] == b'\\' && i + 3 < field.len() {
            let oct = &field[i + 1..i + 4];
            if oct.iter().all(|b| (b'0'..=b'7').contains(b)) {
                let v = oct
                    .iter()
                    .fold(0u32, |acc, b| acc * 8 + u32::from(b - b'0'));
                if let Ok(byte) = u8::try_from(v) {
                    out.push(byte);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(field[i]);
        i += 1;
    }
    out
}

/// Whether `ancestor` is `path` or one of its parent directories.
fn is_ancestor_or_equal(ancestor: &[u8], path: &[u8]) -> bool {
    if ancestor == b"/" {
        return path.first() == Some(&b'/');
    }
    path.starts_with(ancestor) && (path.len() == ancestor.len() || path[ancestor.len()] == b'/')
}

/// Part of `path` below `base` (empty or starting with `/`), if `base` is
/// an ancestor of `path`.
fn relative_to<'a>(base: &[u8], path: &'a [u8]) -> Option<&'a [u8]> {
    if base == b"/" {
        return match path {
            b"/" => Some(&[]),
            p if p.starts_with(b"/") => Some(p),
            _ => None,
        };
    }
    is_ancestor_or_equal(base, path).then(|| &path[base.len()..])
}

/// Joins an absolute `base` with `rel` (empty or starting with `/`).
fn join_abs(base: &[u8], rel: &[u8]) -> Vec<u8> {
    let mut out = base.strip_suffix(b"/").unwrap_or(base).to_vec();
    out.extend_from_slice(rel);
    if out.is_empty() {
        out.push(b'/');
    }
    out
}

impl MountTable {
    /// Reads `/proc/self/mountinfo`; an unreadable file gives an empty table.
    pub fn load() -> Self {
        std::fs::read("/proc/self/mountinfo")
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    /// Parses the contents of a mountinfo file, skipping malformed lines.
    pub fn parse(text: &[u8]) -> Self {
        let mut t = MountTable::default();
        for line in text.split(|&b| b == b'\n') {
            if let Some(e) = Self::parse_line(line) {
                t.by_id.insert(e.id, t.entries.len());
                t.entries.push(e);
            }
        }
        t
    }

    fn parse_line(line: &[u8]) -> Option<MountEntry> {
        let fields: Vec<&[u8]> = line.split(|&b| b == b' ').collect();
        let sep = fields.iter().position(|f| *f == b"-")?;
        if sep < 6 || fields.len() < sep + 3 {
            return None;
        }
        let num = |f: &[u8]| std::str::from_utf8(f).ok()?.parse::<u64>().ok();
        let (major, minor) = {
            let mm = fields[2];
            let colon = mm.iter().position(|&b| b == b':')?;
            (num(&mm[..colon])? as u32, num(&mm[colon + 1..])? as u32)
        };
        Some(MountEntry {
            id: num(fields[0])?,
            parent: num(fields[1])?,
            major,
            minor,
            root: unescape(fields[3]),
            mount_point: unescape(fields[4]),
            fstype: fields[sep + 1].to_vec(),
            source: unescape(fields[sep + 2]),
        })
    }

    /// Entry with mount id `id`.
    pub fn get(&self, id: u64) -> Option<&MountEntry> {
        self.by_id.get(&id).map(|&i| &self.entries[i])
    }

    /// Whether mount `id` is a network filesystem by its type.
    pub fn is_network(&self, id: u64) -> bool {
        self.get(id)
            .is_some_and(|e| NET_TYPES.contains(&e.fstype.as_slice()))
    }

    /// Whether mount `id` shows a part of a filesystem that the scan of
    /// `scan_root` (on mount `root_mnt`) already indexes through another
    /// mount: same device, and its root lies inside the region indexed
    /// through that other mount. Between two mounts with the same root, the
    /// one with the lower id is the original.
    pub fn is_bind_alias(&self, id: u64, scan_root: &[u8], root_mnt: u64) -> bool {
        let Some(e) = self.get(id) else { return false };
        for o in &self.entries {
            if o.id == e.id || (o.major, o.minor) != (e.major, e.minor) {
                continue;
            }
            let region = if o.id == root_mnt {
                let Some(rel) = relative_to(&o.mount_point, scan_root) else {
                    continue;
                };
                join_abs(&o.root, rel)
            } else if is_ancestor_or_equal(scan_root, &o.mount_point) {
                o.root.clone()
            } else {
                continue;
            };
            if !is_ancestor_or_equal(&region, &e.root) {
                continue;
            }
            if region == e.root && o.id > e.id && o.id != root_mnt {
                continue;
            }
            return true;
        }
        false
    }

    /// Rotational flag of the block device behind mount `id`: 0, 1, or 2 if
    /// unknown.
    pub fn rotational(&self, id: u64) -> u8 {
        let Some(e) = self.get(id) else { return 2 };
        if !e.source.starts_with(b"/dev/") {
            return 2;
        }
        let source = std::path::Path::new(std::ffi::OsStr::new(
            std::str::from_utf8(&e.source).unwrap_or(""),
        ));
        let Ok(dev) = std::fs::canonicalize(source) else {
            return 2;
        };
        let Some(name) = dev.file_name() else {
            return 2;
        };
        let base = std::path::Path::new("/sys/class/block").join(name);
        for p in [
            base.join("queue/rotational"),
            base.join("../queue/rotational"),
        ] {
            if let Ok(s) = std::fs::read_to_string(p) {
                return match s.trim() {
                    "0" => 0,
                    "1" => 1,
                    _ => 2,
                };
            }
        }
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"\
22 1 0:21 / /proc rw,nosuid shared:5 - proc proc rw
25 1 0:34 /@ / rw,noatime shared:1 - btrfs /dev/nvme0n1p2 rw,ssd
26 25 0:34 /@home /home rw,noatime shared:2 - btrfs /dev/nvme0n1p2 rw,ssd
30 26 0:34 /@home/u/data /home/u/bind rw shared:2 - btrfs /dev/nvme0n1p2 rw,ssd
31 25 0:50 / /mnt/nas rw - nfs4 nas:/export rw
32 25 0:51 / /mnt/my\\040disk rw - fuse.sshfs u@h:/ rw
33 25 0:60 / /tmp rw - tmpfs tmpfs rw
34 33 0:60 /x/a /tmp/x/b rw - tmpfs tmpfs rw
bad line
";

    #[test]
    fn parses_fields_and_escapes() {
        let t = MountTable::parse(SAMPLE);
        let e = t.get(32).unwrap();
        assert_eq!(e.mount_point, b"/mnt/my disk");
        assert_eq!(e.fstype, b"fuse.sshfs");
        assert_eq!((e.major, e.minor), (0, 51));
        assert!(t.get(99).is_none());
        assert!(t.is_network(31));
        assert!(t.is_network(32));
        assert!(!t.is_network(26));
    }

    #[test]
    fn bind_alias_detection() {
        let t = MountTable::parse(SAMPLE);
        // Scanning /home: the bind of /@home/u/data at /home/u/bind is an alias.
        assert!(t.is_bind_alias(30, b"/home", 26));
        // Scanning /: /home (/@home) is a sibling subvolume, not an alias.
        assert!(!t.is_bind_alias(26, b"/", 25));
        assert!(t.is_bind_alias(30, b"/", 25));
        // Scanning /tmp/x on /tmp: /x/a bound at /tmp/x/b is inside /x.
        assert!(t.is_bind_alias(34, b"/tmp/x", 33));
        // Scanning /tmp/y: /x/a is outside the indexed region /y.
        assert!(!t.is_bind_alias(34, b"/tmp/y", 33));
    }

    #[test]
    fn ancestry() {
        assert!(is_ancestor_or_equal(b"/a", b"/a/b"));
        assert!(is_ancestor_or_equal(b"/a", b"/a"));
        assert!(!is_ancestor_or_equal(b"/a", b"/ab"));
        assert!(is_ancestor_or_equal(b"/", b"/ab"));
    }
}
