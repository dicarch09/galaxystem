// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Benchmark harness (F0-validacion.md §V2).
//!
//! Drives the external tools (hyperfine, GNU time, strace, dua, gdu, du)
//! and summarises their output. Only reads metadata of the measured tree;
//! the only writes are the result files under `bench/results/`.

#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

pub mod env;
pub mod json;
pub mod scan;
pub mod stats;

use std::path::PathBuf;

/// Errors of the benchmark harness.
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    /// An I/O operation on a file or program failed.
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    /// The path to measure is not a directory.
    #[error("{} no es un directorio", .0.display())]
    NotADirectory(PathBuf),
    /// A JSON export could not be read.
    #[error(transparent)]
    Json(#[from] json::JsonError),
    /// The in-process scan failed.
    #[error(transparent)]
    Scan(#[from] gx_scan::ScanError),
    /// An external tool failed or produced unexpected output.
    #[error("{0}")]
    Failed(String),
}
