// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Destination checks (CLAUDE.md "Seguridad", F0-validacion.md §V4).
//!
//! The generator only writes inside a directory that it creates itself, or
//! that already exists and is empty, and only under the temporary prefix
//! unless `allow_path` is given explicitly.

use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::SynthError;

/// Validated destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// Canonical path of the tree root.
    pub root: PathBuf,
    /// Path of the manifest, next to the root (never inside it).
    pub manifest: PathBuf,
    /// Whether the root has to be created (it did not exist).
    pub create: bool,
}

/// Temporary prefix: the canonical form of `std::env::temp_dir()`.
pub fn temp_prefix() -> io::Result<PathBuf> {
    std::fs::canonicalize(std::env::temp_dir())
}

/// Checks `dest` without touching the filesystem except to read metadata.
pub fn check_destination(dest: &Path, allow_path: bool) -> Result<Destination, SynthError> {
    let unsafe_dest = |reason: String| SynthError::UnsafeDestination {
        path: dest.to_path_buf(),
        reason,
    };
    let name = match dest.components().next_back() {
        Some(Component::Normal(name)) => name.to_os_string(),
        _ => return Err(unsafe_dest("must end in a plain directory name".into())),
    };
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = std::fs::canonicalize(parent)
        .map_err(|e| unsafe_dest(format!("parent directory is not accessible: {e}")))?;
    let root = parent.join(&name);

    if !allow_path {
        let prefix =
            temp_prefix().map_err(|e| unsafe_dest(format!("no temporary directory: {e}")))?;
        if !root.starts_with(&prefix) || root == prefix {
            return Err(unsafe_dest(format!(
                "outside the temporary prefix {} (use --allow-path to override)",
                prefix.display()
            )));
        }
    }

    let create = match std::fs::symlink_metadata(&root) {
        Ok(meta) if meta.file_type().is_dir() => {
            let mut entries = std::fs::read_dir(&root)
                .map_err(|e| unsafe_dest(format!("cannot read existing directory: {e}")))?;
            if entries.next().is_some() {
                return Err(unsafe_dest("already exists and is not empty".into()));
            }
            false
        }
        Ok(_) => return Err(unsafe_dest("already exists and is not a directory".into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => true,
        Err(e) => return Err(unsafe_dest(format!("cannot inspect: {e}"))),
    };

    let mut manifest_name = OsString::from(&name);
    manifest_name.push(".manifest.tsv");
    let manifest = parent.join(manifest_name);
    match std::fs::symlink_metadata(&manifest) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        _ => {
            return Err(unsafe_dest(format!(
                "manifest {} already exists",
                manifest.display()
            )));
        }
    }

    Ok(Destination {
        root,
        manifest,
        create,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_new_and_empty_directories_under_tmp() {
        let tmp = tempfile::tempdir().unwrap();
        let d = check_destination(&tmp.path().join("tree"), false).unwrap();
        assert!(d.create);
        assert_eq!(d.manifest.file_name().unwrap(), "tree.manifest.tsv");
        std::fs::create_dir(tmp.path().join("empty")).unwrap();
        assert!(
            !check_destination(&tmp.path().join("empty"), false)
                .unwrap()
                .create
        );
    }

    #[test]
    fn rejects_non_empty_files_and_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let full = tmp.path().join("full");
        std::fs::create_dir(&full).unwrap();
        std::fs::write(full.join("x"), b"x").unwrap();
        assert!(check_destination(&full, false).is_err());
        assert!(check_destination(&full, true).is_err());
        let file = tmp.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(check_destination(&file, false).is_err());
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(tmp.path().join("empty-target"), &link).unwrap();
        std::fs::create_dir(tmp.path().join("empty-target")).unwrap();
        assert!(check_destination(&link, false).is_err());
    }

    #[test]
    fn rejects_paths_outside_tmp_without_allow_path() {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or("/".into());
        let outside = home.join("gx-synth-should-not-exist-3f9a");
        let err = check_destination(&outside, false).unwrap_err();
        assert!(matches!(err, SynthError::UnsafeDestination { .. }));
        assert!(!outside.exists());
        assert!(check_destination(Path::new("/"), true).is_err());
        assert!(check_destination(&std::env::temp_dir(), false).is_err());
        assert!(check_destination(&std::env::temp_dir().join(".."), true).is_err());
    }

    #[test]
    fn rejects_existing_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("t.manifest.tsv"), b"").unwrap();
        assert!(check_destination(&tmp.path().join("t"), false).is_err());
    }
}
