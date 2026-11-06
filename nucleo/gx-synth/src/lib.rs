// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Deterministic synthetic tree generator (F0-diseno.md §6).
//!
//! [`plan`] decides the whole tree from the seed; [`generate`] validates the
//! destination, writes the tree and its manifest. The same seed and
//! parameters always give the same plan.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unsafe_code)]
#![warn(missing_docs)]

mod dist;
mod materialize;
mod plan;
mod safety;

use std::path::{Path, PathBuf};

pub use materialize::{materialize, write_manifest};
pub use plan::{Entry, Kind, PATHOLOGICAL_GIANT, Params, Plan, Profile, plan};
pub use safety::{Destination, check_destination, temp_prefix};

/// Generator errors.
#[derive(Debug, thiserror::Error)]
pub enum SynthError {
    /// The destination failed a safety check; nothing was written.
    #[error("unsafe destination {}: {reason}", path.display())]
    UnsafeDestination {
        /// Requested destination.
        path: PathBuf,
        /// Why it was rejected.
        reason: String,
    },
    /// An I/O operation failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// Path involved.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
}

/// What [`generate`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Canonical root of the tree.
    pub root: PathBuf,
    /// Manifest path.
    pub manifest: PathBuf,
    /// Directories, including the root.
    pub dirs: usize,
    /// Regular files.
    pub files: usize,
    /// Hard links.
    pub hardlinks: usize,
    /// Symlinks.
    pub symlinks: usize,
    /// Fifos.
    pub fifos: usize,
}

/// Validates `dest`, writes the tree described by `params` and its manifest.
pub fn generate(dest: &Path, params: &Params, allow_path: bool) -> Result<Report, SynthError> {
    let d = check_destination(dest, allow_path)?;
    let plan = plan(params);
    if d.create {
        std::fs::create_dir(&d.root).map_err(|source| SynthError::Io {
            path: d.root.clone(),
            source,
        })?;
    }
    materialize(&plan, &d.root, params.allocate)?;
    write_manifest(&plan, &d.root, &d.manifest)?;
    let (dirs, files, hardlinks, symlinks, fifos) = plan.counts();
    Ok(Report {
        root: d.root,
        manifest: d.manifest,
        dirs,
        files,
        hardlinks,
        symlinks,
        fifos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_every_profile_and_is_deterministic() {
        for profile in Profile::ALL {
            let a = tempfile::tempdir().unwrap();
            let b = tempfile::tempdir().unwrap();
            let p = Params::new(profile, 2000, 9);
            let ra = generate(&a.path().join("t"), &p, false).unwrap();
            let rb = generate(&b.path().join("t"), &p, false).unwrap();
            let ma = std::fs::read(&ra.manifest).unwrap();
            let mb = std::fs::read(&rb.manifest).unwrap();
            assert!(!ma.is_empty());
            assert_eq!(ma, mb, "profile {}", profile.name());
            assert_eq!(ra.files, 2000);
            // The manifest lives next to the tree, not inside it.
            assert_eq!(ra.manifest.parent(), ra.root.parent());
        }
    }

    #[test]
    fn allocate_gives_real_blocks() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempfile::tempdir().unwrap();
        let mut p = Params::new(Profile::HomeDev, 50, 2);
        p.allocate = true;
        p.sparse_ratio = 0.0;
        let r = generate(&tmp.path().join("t"), &p, false).unwrap();
        let mut blocks = 0;
        for e in walk(&r.root) {
            let m = std::fs::symlink_metadata(e).unwrap();
            if m.file_type().is_file() {
                blocks += m.blocks();
            }
        }
        assert!(blocks > 0);
    }

    fn walk(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                }
                out.push(e.path());
            }
        }
        out
    }
}
