// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Profiles, parameters and the in-memory tree plan (F0-diseno.md §6).
//!
//! The whole tree is decided here, from the seed alone, before anything is
//! written. Only `Vec`s are used, so iteration order never depends on hashing.

use core::str::FromStr;

use crate::dist::{Fenwick, Rng};

/// Generator profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Many deep `node_modules`, small code files.
    HomeDev,
    /// Flat folders with thousands of 5–50 MB images.
    Fotografo,
    /// Large logs, binary caches, frequent hard links.
    Servidor,
    /// A giant folder, depth 200, non-UTF-8 names and symlink cycles.
    Patologico,
}

impl Profile {
    /// All profiles.
    pub const ALL: [Profile; 4] = [
        Profile::HomeDev,
        Profile::Fotografo,
        Profile::Servidor,
        Profile::Patologico,
    ];

    /// Command-line name.
    pub fn name(self) -> &'static str {
        match self {
            Profile::HomeDev => "home-dev",
            Profile::Fotografo => "fotografo",
            Profile::Servidor => "servidor",
            Profile::Patologico => "patologico",
        }
    }
}

impl FromStr for Profile {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Profile::ALL
            .into_iter()
            .find(|p| p.name() == s)
            .ok_or_else(|| format!("unknown profile {s:?}"))
    }
}

/// Size of the giant folder of `patologico` at full scale.
pub const PATHOLOGICAL_GIANT: u64 = 500_000;

/// Generator parameters. [`Params::new`] fills the defaults of §6 and the
/// profile-specific values; every field can be changed afterwards.
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// Profile.
    pub profile: Profile,
    /// Number of regular files (hard links, symlinks and fifos are extra).
    pub files: u64,
    /// Seed.
    pub seed: u64,
    /// Maximum depth of the general tree.
    pub max_depth: u32,
    /// Pareto shape used for branching and file placement.
    pub pareto_alpha: f64,
    /// Average files per directory; sets the number of directories.
    pub files_per_dir: f64,
    /// Number of giant folders.
    pub giant_dirs: u32,
    /// Files in each giant folder.
    pub giant_size: u64,
    /// Log-normal `mu` of file sizes (bytes, natural log).
    pub size_mu: f64,
    /// Log-normal `sigma` of file sizes.
    pub size_sigma: f64,
    /// Upper bound for a single file size.
    pub max_file_size: u64,
    /// Hard links, as a fraction of `files`.
    pub hardlink_ratio: f64,
    /// Sparse files, as a fraction of `files`.
    pub sparse_ratio: f64,
    /// Non-UTF-8 names, as a fraction of all names.
    pub non_utf8_ratio: f64,
    /// Symlinks, as a fraction of `files`.
    pub symlink_ratio: f64,
    /// Fifos, as a fraction of `files`.
    pub fifo_ratio: f64,
    /// Length of a single nested chain of directories (0 for none).
    pub deep_chain: u32,
    /// Allocate disk blocks with `fallocate` instead of `set_len`.
    pub allocate: bool,
}

impl Params {
    /// Defaults of F0-diseno.md §6 adjusted for `profile`.
    pub fn new(profile: Profile, files: u64, seed: u64) -> Self {
        let mut p = Params {
            profile,
            files,
            seed,
            max_depth: 12,
            pareto_alpha: 1.2,
            files_per_dir: 10.0,
            giant_dirs: 0,
            giant_size: 0,
            size_mu: 9.0,
            size_sigma: 3.0,
            max_file_size: 16 << 30,
            hardlink_ratio: 0.005,
            sparse_ratio: 0.01,
            non_utf8_ratio: 0.001,
            symlink_ratio: 0.01,
            fifo_ratio: 0.0005,
            deep_chain: 0,
            allocate: false,
        };
        match profile {
            Profile::HomeDev => {
                p.files_per_dir = 8.0;
                p.size_mu = 7.5;
                p.size_sigma = 1.8;
            }
            Profile::Fotografo => {
                p.files_per_dir = 400.0;
                p.max_depth = 4;
            }
            Profile::Servidor => {
                p.files_per_dir = 20.0;
                p.size_mu = 10.0;
                p.hardlink_ratio = 0.05;
            }
            Profile::Patologico => {
                p.giant_dirs = 1;
                p.giant_size = PATHOLOGICAL_GIANT.min(files / 2);
                p.deep_chain = 200;
                p.non_utf8_ratio = 0.02;
                p.symlink_ratio = 0.03;
            }
        }
        p
    }
}

/// Kind of a planned entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Directory.
    Dir,
    /// Regular file of `size` apparent bytes.
    File {
        /// Apparent size.
        size: u64,
        /// Leave most of the file as a hole even with `--allocate`.
        sparse: bool,
        /// Executable bit.
        exec: bool,
    },
    /// Hard link to the entry with this index (a file).
    HardLink(usize),
    /// Symlink with this target (never followed).
    Symlink(Vec<u8>),
    /// Named pipe.
    Fifo,
}

/// One planned entry. Entry 0 is the tree root (empty name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Index of the parent directory entry (`usize::MAX` for the root).
    pub parent: usize,
    /// File name.
    pub name: Vec<u8>,
    /// What to create.
    pub kind: Kind,
}

/// Complete plan of a tree, in creation order: directories (parents first),
/// files, fifos, hard links, then symlinks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Entries.
    pub entries: Vec<Entry>,
}

impl Plan {
    /// Relative path of entry `i` (`.` for the root), built from its parents.
    pub fn rel_path(&self, i: usize) -> Vec<u8> {
        let mut chain = Vec::new();
        let mut cur = i;
        while cur != 0 && cur != usize::MAX {
            chain.push(cur);
            cur = self.entries[cur].parent;
        }
        if chain.is_empty() {
            return b".".to_vec();
        }
        let mut out = Vec::new();
        for (k, &c) in chain.iter().rev().enumerate() {
            if k > 0 {
                out.push(b'/');
            }
            out.extend_from_slice(&self.entries[c].name);
        }
        out
    }

    /// Relative symlink target from directory `from` to entry `to`.
    pub fn relative_target(&self, from: usize, to: usize) -> Vec<u8> {
        relative_target(&self.entries, from, to)
    }

    /// Number of entries of each kind: `(dirs, files, hardlinks, symlinks, fifos)`.
    pub fn counts(&self) -> (usize, usize, usize, usize, usize) {
        let mut c = (0, 0, 0, 0, 0);
        for e in &self.entries {
            match e.kind {
                Kind::Dir => c.0 += 1,
                Kind::File { .. } => c.1 += 1,
                Kind::HardLink(_) => c.2 += 1,
                Kind::Symlink(_) => c.3 += 1,
                Kind::Fifo => c.4 += 1,
            }
        }
        c
    }
}

struct Planner<'p> {
    p: &'p Params,
    rng: Rng,
    entries: Vec<Entry>,
    depth: Vec<u32>,
    /// Next unique suffix per directory entry (indexed by entry).
    counter: Vec<u32>,
    has_node_modules: Vec<bool>,
}

const DIR_WORDS: &[&[u8]] = &[
    b"src", b"lib", b"docs", b"data", b"assets", b"build", b"test", b"pkg", b"cache", b"tmp",
];
const STEMS: &[&[u8]] = &[
    b"file", b"index", b"main", b"util", b"notes", b"img", b"log", b"core", b"report", b"mod",
];

impl Planner<'_> {
    fn unique(&mut self, dir: usize) -> u32 {
        let c = self.counter[dir];
        self.counter[dir] += 1;
        c
    }

    fn maybe_non_utf8(&mut self, mut name: Vec<u8>) -> Vec<u8> {
        if self.rng.chance(self.p.non_utf8_ratio) {
            // 0xE9 alone and 0xFF are never valid UTF-8.
            let prefix: &[u8] = if self.rng.chance(0.5) {
                b"caf\xe9-"
            } else {
                b"\xff\xfe-"
            };
            name.splice(0..0, prefix.iter().copied());
        }
        name
    }

    fn push(&mut self, parent: usize, name: Vec<u8>, kind: Kind) -> usize {
        let i = self.entries.len();
        let is_dir = kind == Kind::Dir;
        let d = if parent == usize::MAX {
            0
        } else {
            self.depth[parent] + 1
        };
        self.entries.push(Entry { parent, name, kind });
        self.depth.push(d);
        self.counter.push(0);
        self.has_node_modules.push(false);
        debug_assert!(is_dir || parent != usize::MAX);
        i
    }

    fn dir_name(&mut self, parent: usize) -> Vec<u8> {
        if self.p.profile == Profile::HomeDev
            && !self.has_node_modules[parent]
            && self.rng.chance(0.2)
        {
            self.has_node_modules[parent] = true;
            return b"node_modules".to_vec();
        }
        let word = *self.rng.pick(DIR_WORDS);
        let n = self.unique(parent);
        let name = [word, n.to_string().as_bytes()].concat();
        self.maybe_non_utf8(name)
    }

    fn file_name_and_kind(&mut self, dir: usize) -> (Vec<u8>, Kind) {
        let n = self.unique(dir);
        let (stem, ext, size): (&[u8], &[u8], u64) = match self.p.profile {
            Profile::HomeDev => {
                let ext = *self.rng.pick(&[
                    &b"rs"[..],
                    b"js",
                    b"ts",
                    b"json",
                    b"md",
                    b"py",
                    b"toml",
                    b"c",
                    b"h",
                    b"png",
                    b"lock",
                ]);
                (*self.rng.pick(STEMS), ext, self.size())
            }
            Profile::Fotografo => {
                if self.rng.chance(0.1) {
                    (b"IMG_", b"xmp", 2000 + self.rng.below(8000))
                } else {
                    let ext = *self
                        .rng
                        .pick(&[&b"jpg"[..], b"cr2", b"nef", b"arw", b"dng"]);
                    // Log-uniform between 5 and 50 MB.
                    let (lo, hi) = (5.0e6f64.ln(), 50.0e6f64.ln());
                    let size = (lo + (hi - lo) * self.rng.unit()).exp() as u64;
                    (b"IMG_", ext, size)
                }
            }
            Profile::Servidor => {
                let ext = *self
                    .rng
                    .pick(&[&b"log"[..], b"gz", b"bin", b"cache", b"db", b"conf"]);
                (*self.rng.pick(STEMS), ext, self.size())
            }
            Profile::Patologico => {
                let ext = *self.rng.pick(&[&b"txt"[..], b"dat", b"tar.gz", b"x"]);
                (*self.rng.pick(STEMS), ext, self.size())
            }
        };
        let exec = self.rng.chance(0.02);
        let mut name = [stem, n.to_string().as_bytes()].concat();
        if !exec {
            name.push(b'.');
            name.extend_from_slice(ext);
        }
        let name = self.maybe_non_utf8(name);
        let sparse = self.rng.chance(self.p.sparse_ratio);
        (name, Kind::File { size, sparse, exec })
    }

    fn size(&mut self) -> u64 {
        let s = self.rng.log_normal(self.p.size_mu, self.p.size_sigma);
        (s as u64).min(self.p.max_file_size)
    }

    fn run(mut self) -> Plan {
        let p = self.p;
        self.push(usize::MAX, Vec::new(), Kind::Dir);

        // General directory tree: preferential attachment with Pareto weights.
        let giant_files = u64::from(p.giant_dirs) * p.giant_size;
        let spread_files = p.files.saturating_sub(giant_files);
        let n_dirs = ((spread_files as f64 / p.files_per_dir).round() as usize).max(1);
        let capacity = n_dirs + p.deep_chain as usize + p.giant_dirs as usize + 2;
        let mut branch = Fenwick::with_capacity(capacity);
        let mut dirs = vec![0usize];
        branch.push(self.rng.pareto(p.pareto_alpha));
        for _ in 1..n_dirs {
            let parent = dirs[branch.sample(&mut self.rng)];
            let name = self.dir_name(parent);
            let d = self.push(parent, name, Kind::Dir);
            let w = if self.depth[d] < p.max_depth {
                self.rng.pareto(p.pareto_alpha)
            } else {
                0.0
            };
            dirs.push(d);
            branch.push(w);
        }

        // Deep chain.
        let mut chain_dirs = Vec::new();
        if p.deep_chain > 0 {
            let mut parent = *self.rng.pick(&dirs);
            for _ in 0..p.deep_chain {
                let n = self.unique(parent);
                parent = self.push(parent, format!("d{n}").into_bytes(), Kind::Dir);
                chain_dirs.push(parent);
            }
        }

        // Giant folders.
        let mut giants = Vec::new();
        for _ in 0..p.giant_dirs {
            let parent = *self.rng.pick(&dirs);
            let n = self.unique(parent);
            giants.push(self.push(parent, format!("giant{n}").into_bytes(), Kind::Dir));
        }

        // File placement weights over all non-giant directories.
        let targets: Vec<usize> = dirs.iter().chain(&chain_dirs).copied().collect();
        let mut place = Fenwick::with_capacity(targets.len());
        for _ in &targets {
            place.push(self.rng.pareto(p.pareto_alpha));
        }

        let mut files = Vec::with_capacity(p.files as usize);
        for &g in &giants {
            for _ in 0..p.giant_size {
                let (name, kind) = self.file_name_and_kind(g);
                files.push(self.push(g, name, kind));
            }
        }
        for _ in 0..spread_files {
            let dir = targets[place.sample(&mut self.rng)];
            let (name, kind) = self.file_name_and_kind(dir);
            files.push(self.push(dir, name, kind));
        }

        let count = |ratio: f64| (p.files as f64 * ratio).round() as u64;

        for _ in 0..count(p.fifo_ratio) {
            let dir = targets[place.sample(&mut self.rng)];
            let n = self.unique(dir);
            self.push(dir, format!("fifo{n}").into_bytes(), Kind::Fifo);
        }

        if !files.is_empty() {
            for _ in 0..count(p.hardlink_ratio) {
                let target = *self.rng.pick(&files);
                let dir = targets[place.sample(&mut self.rng)];
                let n = self.unique(dir);
                self.push(dir, format!("hl{n}").into_bytes(), Kind::HardLink(target));
            }
        }

        let mut remaining = count(p.symlink_ratio);
        let existing = self.entries.len();
        while remaining > 0 && existing > 1 {
            let dir = targets[place.sample(&mut self.rng)];
            let roll = self.rng.unit();
            let n = self.unique(dir);
            let name = format!("ln{n}").into_bytes();
            if roll < 0.40 {
                // To a random existing entry (file or directory).
                let to = 1 + self.rng.below(existing as u64 - 1) as usize;
                let target = relative_target(&self.entries, dir, to);
                self.push(dir, name, Kind::Symlink(target));
            } else if roll < 0.60 {
                // To an ancestor: a directory cycle.
                let up = 1 + self.rng.below(u64::from(self.depth[dir]) + 1);
                let target = b"../".repeat(up as usize);
                self.push(
                    dir,
                    name,
                    Kind::Symlink(target[..target.len() - 1].to_vec()),
                );
            } else if roll < 0.72 {
                self.push(dir, name, Kind::Symlink(format!("missing{n}").into_bytes()));
            } else if roll < 0.84 && remaining >= 2 {
                // Two links pointing at each other.
                let m = self.unique(dir);
                let other = format!("ln{m}").into_bytes();
                self.push(dir, name.clone(), Kind::Symlink(other.clone()));
                self.push(dir, other, Kind::Symlink(name));
                remaining -= 1;
            } else if roll < 0.92 || p.profile != Profile::Patologico {
                // Self loop.
                self.push(dir, name.clone(), Kind::Symlink(name));
            } else {
                self.push(dir, name, Kind::Symlink(b"/".to_vec()));
            }
            remaining -= 1;
        }

        Plan {
            entries: self.entries,
        }
    }
}

/// Entry indices from the root down to `i`.
fn chain(entries: &[Entry], i: usize) -> Vec<usize> {
    let mut c = vec![i];
    let mut cur = i;
    while cur != 0 {
        cur = entries[cur].parent;
        c.push(cur);
    }
    c.reverse();
    c
}

/// Relative path from directory `from` to entry `to`.
fn relative_target(entries: &[Entry], from: usize, to: usize) -> Vec<u8> {
    let a = chain(entries, from);
    let b = chain(entries, to);
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<&[u8]> = vec![b".."; a.len() - common];
    parts.extend(b[common..].iter().map(|&c| entries[c].name.as_slice()));
    if parts.is_empty() {
        return b".".to_vec();
    }
    parts.join(&b'/')
}

/// Builds the plan for `params`. Same parameters, same plan.
pub fn plan(params: &Params) -> Plan {
    Planner {
        p: params,
        rng: Rng::new(params.seed),
        entries: Vec::new(),
        depth: Vec::new(),
        counter: Vec::new(),
        has_node_modules: Vec::new(),
    }
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_are_deterministic() {
        for profile in Profile::ALL {
            let p = Params::new(profile, 3000, 42);
            assert_eq!(plan(&p), plan(&p));
            let mut other = p.clone();
            other.seed = 43;
            assert_ne!(plan(&p), plan(&other));
        }
    }

    #[test]
    fn names_are_unique_within_each_directory() {
        for profile in Profile::ALL {
            let pl = plan(&Params::new(profile, 5000, 1));
            let mut seen = std::collections::HashSet::new();
            for e in &pl.entries[1..] {
                assert!(
                    seen.insert((e.parent, e.name.clone())),
                    "duplicate {:?}",
                    e.name
                );
                assert!(!e.name.is_empty() && e.name.len() <= 255);
                assert!(!e.name.contains(&b'/') && !e.name.contains(&0));
            }
        }
    }

    #[test]
    fn counts_follow_parameters() {
        let p = Params::new(Profile::Servidor, 10_000, 5);
        let (_, files, hardlinks, symlinks, fifos) = plan(&p).counts();
        assert_eq!(files, 10_000);
        assert_eq!(hardlinks, 500);
        assert_eq!(symlinks, 100);
        assert_eq!(fifos, 5);
    }

    #[test]
    fn pathological_profile_has_its_features() {
        let pl = plan(&Params::new(Profile::Patologico, 4000, 1));
        let max_depth = (0..pl.entries.len())
            .map(|i| pl.rel_path(i).iter().filter(|&&b| b == b'/').count())
            .max()
            .unwrap();
        assert!(max_depth >= 200);
        let giant = pl
            .entries
            .iter()
            .position(|e| e.name.starts_with(b"giant"))
            .unwrap();
        let in_giant = pl.entries.iter().filter(|e| e.parent == giant).count();
        assert_eq!(in_giant, 2000);
        assert!(
            pl.entries
                .iter()
                .any(|e| std::str::from_utf8(&e.name).is_err())
        );
        assert!(
            pl.entries
                .iter()
                .any(|e| e.kind == Kind::Symlink(b"/".to_vec()))
        );
    }

    #[test]
    fn relative_targets() {
        let pl = plan(&Params::new(Profile::HomeDev, 200, 1));
        // Every non-root entry's path resolves through its parent chain.
        assert_eq!(pl.rel_path(0), b".");
        let leaf = pl.entries.len() - 1;
        let parent = pl.entries[leaf].parent;
        assert_eq!(pl.relative_target(parent, parent), b".");
    }
}
