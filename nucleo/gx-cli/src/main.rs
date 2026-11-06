// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `gx` command-line tool (F0-diseno.md §8).

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(unsafe_code)]
#![warn(missing_docs)]

mod verify;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use gx_scan::{NoProgress, ScanMode, ScanOptions, ThreadPolicy};
use gx_synth::{Params, Profile};

/// Galaxystem indexer tools.
#[derive(Parser)]
#[command(
    name = "gx",
    version,
    about = "Herramientas del indexador de Galaxystem"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Scanner options shared by `scan` and `verify-du`.
#[derive(Args)]
struct ScanArgs {
    /// Usa el escáner de un hilo de la F0.1 (referencia).
    #[arg(long)]
    sequential: bool,
    /// Hilos por sistema de archivos (por defecto según el tipo de disco).
    #[arg(long, conflicts_with = "sequential")]
    threads: Option<usize>,
}

impl ScanArgs {
    fn options(&self) -> ScanOptions {
        let mut opts = ScanOptions::default();
        if self.sequential {
            opts.mode = ScanMode::Sequential;
        }
        if let Some(n) = self.threads {
            opts.threads = ThreadPolicy::uniform(n.max(1));
        }
        opts
    }
}

#[derive(Subcommand)]
enum Command {
    /// Escanea una ruta e imprime totales, nodos, tiempo y bytes por nodo.
    Scan {
        /// Ruta a escanear.
        path: PathBuf,
        /// Muestra el tamaño aparente como total principal.
        #[arg(long)]
        apparent: bool,
        #[command(flatten)]
        scan: ScanArgs,
    },
    /// Compara el escaneo con du y find (F0-validacion.md §V1).
    VerifyDu {
        /// Ruta a verificar.
        path: PathBuf,
        #[command(flatten)]
        scan: ScanArgs,
    },
    /// Genera un árbol sintético determinista (F0-diseno.md §6).
    Synth {
        /// Perfil: home-dev, fotografo, servidor o patologico.
        #[arg(long, default_value = "home-dev")]
        profile: Profile,
        /// Cantidad de archivos regulares.
        #[arg(long, default_value_t = 100_000)]
        files: u64,
        /// Semilla.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Reserva bloques con fallocate en vez de set_len.
        #[arg(long)]
        allocate: bool,
        /// Archivos de la carpeta gigante (por defecto según el perfil).
        #[arg(long)]
        giant_size: Option<u64>,
        /// Permite un destino fuera del directorio temporal.
        #[arg(long)]
        allow_path: bool,
        /// Directorio destino (nuevo o vacío).
        dest: PathBuf,
    },
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.2} {}", UNITS[u])
}

fn scan(path: &Path, apparent: bool, opts: &ScanOptions) -> Result<()> {
    let (arena, stats) = gx_scan::scan_with_stats(path, opts, &NoProgress)
        .with_context(|| format!("no se pudo escanear {}", path.display()))?;
    let root = arena.root();
    let (disk, app) = (arena.disk_bytes(root), arena.apparent_bytes(root));
    let mem = arena.memory_usage();
    println!(
        "Raíz:             {}",
        String::from_utf8_lossy(&stats.root_path)
    );
    if apparent {
        println!("Tamaño aparente:  {app} B ({})", human(app));
        println!("Uso en disco:     {disk} B ({})", human(disk));
    } else {
        println!("Uso en disco:     {disk} B ({})", human(disk));
        println!("Tamaño aparente:  {app} B ({})", human(app));
    }
    println!(
        "Nodos:            {} ({} carpetas, {} archivos, {} symlinks, {} especiales)",
        stats.nodes, stats.dirs, stats.files, stats.symlinks, stats.specials
    );
    let first = stats
        .time_to_10k
        .map(|t| format!("{:.1} ms", t.as_secs_f64() * 1e3))
        .unwrap_or_else(|| "no alcanzado".into());
    println!(
        "Tiempo:           {:.3} s (primeros 10.000 nodos: {first}; {} hilos)",
        stats.elapsed.as_secs_f64(),
        stats.threads
    );
    println!(
        "Bytes por nodo:   {:.1} (calientes {} + frías {} + nombres {} + carpetas {} + resúmenes {} B)",
        mem.total() as f64 / stats.nodes.max(1) as f64,
        mem.hot,
        mem.cold,
        mem.names,
        mem.dirs,
        mem.summaries
    );
    println!(
        "Incidencias:      {} bloqueadas, {} errores, {} desaparecidas, {} hard links repetidos, {} montajes, {} fronteras de dispositivo, {} sin respuesta de red",
        stats.locked,
        stats.errors,
        stats.vanished,
        stats.hardlink_dups,
        stats.mount_points,
        stats.device_boundaries,
        stats.timed_out
    );
    for e in stats.error_log.iter().take(10) {
        println!(
            "  {} {}: errno {}",
            e.op,
            String::from_utf8_lossy(&e.path),
            e.errno
        );
    }
    Ok(())
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Scan {
            path,
            apparent,
            scan: args,
        } => scan(&path, apparent, &args.options()).map(|()| ExitCode::SUCCESS),
        Command::VerifyDu { path, scan: args } => Ok(if verify::run(&path, &args.options())?.ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }),
        Command::Synth {
            profile,
            files,
            seed,
            allocate,
            giant_size,
            allow_path,
            dest,
        } => {
            let mut params = Params::new(profile, files, seed);
            params.allocate = allocate;
            if let Some(g) = giant_size {
                params.giant_size = g;
            }
            let r = gx_synth::generate(&dest, &params, allow_path)?;
            println!("Árbol:      {}", r.root.display());
            println!("Manifiesto: {}", r.manifest.display());
            println!(
                "Entradas:   {} carpetas, {} archivos, {} hard links, {} symlinks, {} fifos",
                r.dirs, r.files, r.hardlinks, r.symlinks, r.fifos
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("gx: error: {e:#}");
            ExitCode::from(2)
        }
    }
}
