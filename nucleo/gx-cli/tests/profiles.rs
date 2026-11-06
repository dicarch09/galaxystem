// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! End-to-end checks through the `gx` binary: every profile against
//! `gx verify-du` (F0-validacion.md §V1) and generator determinism.

use std::path::Path;
use std::process::Command;

const PROFILES: [&str; 4] = ["home-dev", "fotografo", "servidor", "patologico"];

/// Runs the `gx` binary, through the cargo target runner (for example
/// `qemu-aarch64` when cross-testing) if one is configured.
fn gx() -> Command {
    let bin = env!("CARGO_BIN_EXE_gx");
    let runner = std::env::vars()
        .find(|(k, v)| k.starts_with("CARGO_TARGET_") && k.ends_with("_RUNNER") && !v.is_empty())
        .map(|(_, v)| v);
    match runner {
        Some(r) => {
            let mut parts = r.split_whitespace();
            let mut cmd = Command::new(parts.next().unwrap_or_default());
            cmd.args(parts).arg(bin);
            cmd
        }
        None => Command::new(bin),
    }
}

fn synth(profile: &str, files: u64, seed: u64, dest: &Path, extra: &[&str]) {
    let out = gx()
        .args(["synth", "--profile", profile, "--files"])
        .arg(files.to_string())
        .arg("--seed")
        .arg(seed.to_string())
        .args(extra)
        .arg(dest)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gx synth failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn verify(profile: &str, files: u64, extra: &[&str]) {
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("tree");
    synth(profile, files, 1, &tree, extra);
    let out = gx().arg("verify-du").arg(&tree).output().unwrap();
    let report = String::from_utf8_lossy(&out.stdout);
    println!("--- {profile} {files} {extra:?}\n{report}");
    assert!(
        out.status.success(),
        "verify-du failed for {profile}:\n{report}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn manifests_identical(profile: &str, files: u64) {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    synth(profile, files, 7, &a.path().join("tree"), &[]);
    synth(profile, files, 7, &b.path().join("tree"), &[]);
    let ma = std::fs::read(a.path().join("tree.manifest.tsv")).unwrap();
    let mb = std::fs::read(b.path().join("tree.manifest.tsv")).unwrap();
    assert!(ma.len() > files as usize);
    assert!(ma == mb, "manifests differ for {profile}");
}

#[test]
fn small_profiles_match_du() {
    for p in PROFILES {
        verify(p, 3_000, &[]);
    }
}

#[test]
fn small_generation_is_deterministic() {
    for p in PROFILES {
        manifests_identical(p, 3_000);
    }
}

#[test]
fn synth_rejects_unsafe_destinations() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("busy"), b"").unwrap();
    // Existing, non-empty directory.
    let out = gx()
        .arg("synth")
        .arg("--files")
        .arg("10")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    // Outside the temporary prefix.
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/".into());
    let outside = Path::new(&home).join("gx-synth-must-not-exist-81c2");
    let out = gx()
        .arg("synth")
        .arg("--files")
        .arg("10")
        .arg(&outside)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!outside.exists());
}

#[test]
#[ignore = "100 000 archivos por perfil; ejecutar con --ignored"]
fn profiles_100k_match_du() {
    for p in PROFILES {
        verify(p, 100_000, &[]);
    }
}

#[test]
#[ignore = "100 000 archivos por perfil; ejecutar con --ignored"]
fn generation_100k_is_deterministic() {
    for p in PROFILES {
        manifests_identical(p, 100_000);
    }
}

#[test]
#[ignore = "100 000 archivos con bloques reales; ejecutar con --ignored"]
fn home_dev_100k_allocated_matches_du() {
    let st = rustix::fs::statvfs(std::env::temp_dir()).unwrap();
    let free = st.f_bavail * st.f_frsize;
    if free < 4 << 30 {
        eprintln!(
            "AVISO: prueba omitida: {} MiB libres en el directorio temporal, hacen falta 4096",
            free >> 20
        );
        return;
    }
    verify("home-dev", 100_000, &["--allocate"]);
}
