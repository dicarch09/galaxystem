// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `gx verify-du`: accuracy checks of F0-validacion.md §V1.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use gx_arena::{Arena, Flags, NodeId};
use gx_scan::{NoProgress, ScanOptions, ScanStats};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::{Rng as _, SeedableRng};

/// Fixed seed for the folder sample (§V1.2).
const SAMPLE_SEED: u64 = 0x4758_5644_5531;
/// Folders compared individually.
const SAMPLE_SIZE: usize = 200;

/// Outcome of the three checks.
pub struct Outcome {
    /// Whether everything matched.
    pub ok: bool,
}

fn first_line(cmd: &str) -> String {
    Command::new(cmd)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("{cmd}: versión desconocida"))
}

/// Runs `du -x -s` with `-B1` (disk) or `-b` (apparent) on `path`.
fn du(path: &OsStr, apparent: bool) -> Result<u64> {
    let out = Command::new("du")
        .args(["-x", "-s", if apparent { "-b" } else { "-B1" }, "--"])
        .arg(path)
        .output()
        .context("no se pudo ejecutar du")?;
    // du exits with 1 when it cannot read a locked folder but still prints
    // the total, which is what we compare.
    let stdout = out.stdout;
    let field = stdout
        .split(|&b| b == b'\t' || b == b' ' || b == b'\n')
        .next()
        .filter(|f| !f.is_empty());
    match field.and_then(|f| std::str::from_utf8(f).ok()?.parse().ok()) {
        Some(v) => Ok(v),
        None => bail!(
            "salida inesperada de du para {}: {}",
            Path::new(path).display(),
            String::from_utf8_lossy(&out.stderr)
        ),
    }
}

/// Counts entries with `find <root> -xdev -printf .` (robust to newlines in
/// names).
fn find_count(root: &OsStr) -> Result<u64> {
    let out = Command::new("find")
        .arg(root)
        .args(["-xdev", "-printf", "."])
        .output()
        .context("no se pudo ejecutar find")?;
    Ok(out.stdout.iter().filter(|&&b| b == b'.').count() as u64)
}

/// Nodes `find -xdev` should list: everything not strictly below a mount
/// point or device boundary (pseudo-fs, bind aliases and locked folders have
/// no indexed descendants anyway).
fn expected_find_count(a: &Arena) -> u64 {
    let n = a.len();
    let mut hidden = vec![false; n];
    let mut count = 0;
    for i in 0..n {
        let id = NodeId(i as u32);
        if let Some(p) = a.parent(id) {
            let p_stops = p.0 != 0 && is_boundary(a, p);
            hidden[i] = hidden[p.0 as usize] || p_stops;
        }
        if !hidden[i] {
            count += 1;
        }
    }
    count
}

fn is_boundary(a: &Arena, n: NodeId) -> bool {
    match a.parent(n) {
        None => false,
        Some(p) => a.flags(n).contains(Flags::MOUNT_POINT) || a.dev(n) != a.dev(p),
    }
}

/// Directories eligible for the per-folder comparison: no hard link in the
/// subtree and not pseudo-fs, alias, error or stale.
fn candidates(a: &Arena, stats: &ScanStats) -> Vec<NodeId> {
    let n = a.len();
    let mut linked = vec![false; n];
    for &m in &stats.multi_link_nodes {
        let mut cur = Some(m);
        while let Some(c) = cur {
            if linked[c.0 as usize] {
                break;
            }
            linked[c.0 as usize] = true;
            cur = a.parent(c);
        }
    }
    let skip = Flags::PSEUDO_FS | Flags::BIND_ALIAS | Flags::ERROR | Flags::STALE;
    (1..n as u32)
        .map(NodeId)
        .filter(|&d| {
            let f = a.flags(d);
            f.contains(Flags::DIR) && !f.intersects(skip) && !linked[d.0 as usize]
        })
        .collect()
}

fn sample(mut items: Vec<NodeId>, k: usize) -> Vec<NodeId> {
    let mut rng = Xoshiro256PlusPlus::seed_from_u64(SAMPLE_SEED);
    let k = k.min(items.len());
    for i in 0..k {
        let span = (items.len() - i) as u64;
        let j = i + ((u128::from(rng.next_u64()) * u128::from(span)) >> 64) as usize;
        items.swap(i, j);
    }
    items.truncate(k);
    items
}

fn mark(ok: bool) -> &'static str {
    if ok { "OK" } else { "DIFERENTE" }
}

/// Runs §V1 on `root` and prints a summary.
pub fn run(root: &Path, opts: &ScanOptions) -> Result<Outcome> {
    let (arena, stats) = gx_scan::scan_with_stats(root, opts, &NoProgress)
        .with_context(|| format!("no se pudo escanear {}", root.display()))?;
    arena
        .check_invariants()
        .map_err(|e| anyhow::anyhow!("invariante violada: {e}"))?;
    let root_os = OsStr::from_bytes(&stats.root_path).to_owned();
    let mut ok = true;

    println!("Raíz: {}", Path::new(&root_os).display());
    println!(
        "Herramientas: {} | {}",
        first_line("du"),
        first_line("find")
    );
    println!("Nodos indexados: {}", arena.len());

    // 1. Root totals.
    let du_disk = du(&root_os, false)?;
    let du_app = du(&root_os, true)?;
    let gx_disk = arena.disk_bytes(arena.root());
    let gx_app = arena.apparent_bytes(arena.root());
    println!("\n1. Totales de la raíz");
    println!(
        "   uso en disco    gx {gx_disk:>16}  du {du_disk:>16}  {}",
        mark(gx_disk == du_disk)
    );
    println!(
        "   tamaño aparente gx {gx_app:>16}  du {du_app:>16}  {}",
        mark(gx_app == du_app)
    );
    ok &= gx_disk == du_disk && gx_app == du_app;

    // 2. Sampled folders.
    let cands = candidates(&arena, &stats);
    let picked = sample(cands.clone(), SAMPLE_SIZE);
    let mut mismatches = Vec::new();
    let mut path = Vec::new();
    for &d in &picked {
        arena.path(d, &mut path);
        let p = OsStr::from_bytes(&path);
        let (dd, da) = (du(p, false)?, du(p, true)?);
        let (gd, ga) = (arena.disk_bytes(d), arena.apparent_bytes(d));
        if dd != gd || da != ga {
            mismatches.push((path.clone(), gd, dd, ga, da));
        }
    }
    println!(
        "\n2. Carpetas al azar (seed {SAMPLE_SEED:#x}): {} comparadas de {} candidatas, {} diferentes",
        picked.len(),
        cands.len(),
        mismatches.len()
    );
    for (p, gd, dd, ga, da) in mismatches.iter().take(20) {
        println!(
            "   {}: disco gx {gd} du {dd}, aparente gx {ga} du {da}",
            String::from_utf8_lossy(p)
        );
    }
    ok &= mismatches.is_empty();

    // 3. Entry count.
    let found = find_count(&root_os)?;
    let expected = expected_find_count(&arena);
    println!("\n3. Conteo de entradas");
    println!(
        "   gx {expected:>12}  find -xdev {found:>12}  {}",
        mark(expected == found)
    );
    ok &= expected == found;

    println!(
        "\nResultado: {}",
        if ok {
            "todo coincide"
        } else {
            "HAY DIFERENCIAS"
        }
    );
    Ok(Outcome { ok })
}
