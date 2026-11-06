// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `statfs.f_type` values (F0-diseno.md §2.4), from `<linux/magic.h>`.

/// btrfs.
pub const BTRFS: u64 = 0x9123_683E;

/// Filesystems whose contents are never scanned.
pub const PSEUDO: &[(u64, &str)] = &[
    (0x9fa0, "proc"),
    (0x6265_6572, "sysfs"),
    (0x0027_e0eb, "cgroup"),
    (0x6367_7270, "cgroup2"),
    (0x6462_6720, "debugfs"),
    (0x7472_6163, "tracefs"),
    (0x7363_6673, "securityfs"),
    (0xcafe_4a11, "bpf"),
    (0x6165_676C, "pstore"),
    (0x6265_6570, "configfs"),
    (0x1cd1, "devpts"),
    (0x1980_0202, "mqueue"),
    (0x9584_58f6, "hugetlbfs"),
    (0x6573_5543, "fusectl"),
];

/// Network filesystems recognisable by magic.
pub const NETWORK: &[(u64, &str)] = &[
    (0x6969, "nfs"),
    (0xFF53_4D42, "cifs"),
    (0xFE53_4D42, "smb2"),
];

/// Normalises a raw `f_type` (a signed `long`) to its 32-bit magic.
pub fn normalize(f_type: i64) -> u64 {
    (f_type as u64) & 0xFFFF_FFFF
}

/// Whether `magic` is a pseudo filesystem.
pub fn is_pseudo(magic: u64) -> bool {
    PSEUDO.iter().any(|&(m, _)| m == magic)
}

/// Whether `magic` is a network filesystem.
pub fn is_network(magic: u64) -> bool {
    NETWORK.iter().any(|&(m, _)| m == magic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert!(is_pseudo(0x9fa0));
        assert!(!is_pseudo(BTRFS));
        assert!(is_network(normalize(0xFF53_4D42u32 as i32 as i64)));
        // tmpfs is scanned normally.
        assert!(!is_pseudo(0x0102_1994));
    }
}
