// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writes a [`Plan`] to disk and produces its manifest.

use std::ffi::OsStr;
use std::fs::{DirBuilder, OpenOptions};
use std::io::{BufWriter, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use rustix::fs::{FallocateFlags, FileType, Mode};

use crate::SynthError;
use crate::plan::{Kind, Plan};

/// Bytes really allocated at the start of a sparse file with `--allocate`.
const SPARSE_ALLOCATED: u64 = 4096;

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> SynthError + '_ {
    move |source| SynthError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Creates every entry of `plan` under `root`, which must already exist.
///
/// Symlinks are created last, so no path used while creating the tree ever
/// goes through one. Files are opened with `O_CREAT | O_EXCL | O_NOFOLLOW`.
pub fn materialize(plan: &Plan, root: &Path, allocate: bool) -> Result<(), SynthError> {
    let mut paths: Vec<PathBuf> = Vec::with_capacity(plan.entries.len());
    paths.push(root.to_path_buf());
    for (i, e) in plan.entries.iter().enumerate().skip(1) {
        let path = paths[e.parent].join(OsStr::from_bytes(&e.name));
        match &e.kind {
            Kind::Dir => DirBuilder::new()
                .mode(0o755)
                .create(&path)
                .map_err(io_err(&path))?,
            Kind::File { size, sparse, exec } => {
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                    .mode(if *exec { 0o755 } else { 0o644 })
                    .open(&path)
                    .map_err(io_err(&path))?;
                if allocate && *size > 0 {
                    let alloc = if *sparse {
                        SPARSE_ALLOCATED.min(*size / 2)
                    } else {
                        *size
                    };
                    if alloc > 0 {
                        rustix::fs::fallocate(&file, FallocateFlags::empty(), 0, alloc).map_err(
                            |e| SynthError::Io {
                                path: path.clone(),
                                source: e.into(),
                            },
                        )?;
                    }
                }
                file.set_len(*size).map_err(io_err(&path))?;
            }
            Kind::Fifo => rustix::fs::mknodat(
                rustix::fs::CWD,
                &path,
                FileType::Fifo,
                Mode::from_raw_mode(0o644),
                0,
            )
            .map_err(|e| SynthError::Io {
                path: path.clone(),
                source: e.into(),
            })?,
            Kind::HardLink(target) => {
                std::fs::hard_link(&paths[*target], &path).map_err(io_err(&path))?
            }
            Kind::Symlink(target) => std::os::unix::fs::symlink(OsStr::from_bytes(target), &path)
                .map_err(io_err(&path))?,
        }
        debug_assert_eq!(paths.len(), i);
        paths.push(path);
    }
    Ok(())
}

/// Writes `rel_path \t type \t apparent_size` for every entry, in plan
/// order, from `lstat` of what is actually on disk. Bytes outside printable
/// ASCII, `\` and tabs are written as `\xHH`.
pub fn write_manifest(plan: &Plan, root: &Path, manifest: &Path) -> Result<(), SynthError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .mode(0o644)
        .open(manifest)
        .map_err(io_err(manifest))?;
    let mut out = BufWriter::new(file);
    let mut line = Vec::new();
    for i in 0..plan.entries.len() {
        let rel = plan.rel_path(i);
        let path = root.join(OsStr::from_bytes(&rel));
        let meta = std::fs::symlink_metadata(&path).map_err(io_err(&path))?;
        let ft = meta.file_type();
        let kind = if ft.is_dir() {
            "dir"
        } else if ft.is_symlink() {
            "symlink"
        } else if ft.is_file() {
            "file"
        } else {
            "special"
        };
        line.clear();
        escape_into(&rel, &mut line);
        line.extend_from_slice(format!("\t{kind}\t{}\n", meta.size()).as_bytes());
        out.write_all(&line).map_err(io_err(manifest))?;
    }
    out.flush().map_err(io_err(manifest))?;
    Ok(())
}

fn escape_into(bytes: &[u8], out: &mut Vec<u8>) {
    for &b in bytes {
        if (0x21..0x7f).contains(&b) && b != b'\\' || b == b' ' {
            out.push(b);
        } else {
            out.extend_from_slice(format!("\\x{b:02x}").as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        let mut v = Vec::new();
        escape_into(b"a b\t\\\xff", &mut v);
        assert_eq!(v, b"a b\\x09\\x5c\\xff");
    }
}
