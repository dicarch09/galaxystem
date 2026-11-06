// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `gx bench scan`: the scan measurements of F0-validacion.md §V2.
//!
//! With a warm cache it runs hyperfine over gx (both `statx` backends when
//! available), dua, gdu and `du`; then one run of each under
//! `/usr/bin/time -v` and another under `strace -c -f`; and finally the
//! scanner in-process to time the first 10 000 nodes. Results go to
//! `<out>/<machine>/<date>/`. Cold-cache runs need root, so their commands
//! are printed instead of run.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gx_scan::{NoProgress, ScanOptions, StatxBackend};

use crate::env::{self, command_line, quote};
use crate::stats::{Timing, hyperfine_timings, median, percentile};
use crate::{BenchError, json};

/// Configuration of a scan benchmark.
#[derive(Clone, Debug)]
pub struct ScanBench {
    /// Directory to scan.
    pub path: PathBuf,
    /// The `gx` binary to measure.
    pub gx: PathBuf,
    /// Whether `gx` was built with the `uring` feature.
    pub uring: bool,
    /// Machine name, used in the results path.
    pub machine: String,
    /// Base results directory (`bench/results`).
    pub out: PathBuf,
    /// Measured runs per command.
    pub runs: u32,
    /// Warm-up runs per command.
    pub warmup: u32,
    /// Only print the commands, run nothing.
    pub print_only: bool,
}

/// One command under measurement.
#[derive(Clone, Debug)]
struct Tool {
    name: &'static str,
    version: String,
    argv: Vec<String>,
}

/// Resources of one run under `/usr/bin/time -v`.
#[derive(Clone, Copy, Debug, Default)]
struct Resources {
    max_rss_kib: Option<u64>,
    user_s: Option<f64>,
    system_s: Option<f64>,
}

/// In-process timings of the scanner.
#[derive(Clone, Debug)]
struct InProcess {
    label: &'static str,
    nodes: u64,
    first_10k: Vec<f64>,
    total: Vec<f64>,
    bytes_per_node: f64,
    columns_and_names_per_node: f64,
    uring_threads: u64,
    uring_error: Option<i32>,
}

/// Parses `/usr/bin/time -v` output.
fn parse_time_v(text: &str) -> Resources {
    let field = |label: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(label))
            .map(|v| v.trim().to_owned())
    };
    Resources {
        max_rss_kib: field("Maximum resident set size (kbytes):").and_then(|v| v.parse().ok()),
        user_s: field("User time (seconds):").and_then(|v| v.parse().ok()),
        system_s: field("System time (seconds):").and_then(|v| v.parse().ok()),
    }
}

/// Total syscall count from `strace -c` output (the `total` row).
fn parse_strace_total(text: &str) -> Option<u64> {
    let line = text
        .lines()
        .rev()
        .find(|l| l.trim_end().ends_with("total"))?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    // % time, seconds, usecs/call, calls, [errors], "total".
    fields.get(3)?.parse().ok()
}

fn first_existing_free(base: &Path) -> PathBuf {
    let empty = |p: &Path| fs::read_dir(p).map_or(true, |mut d| d.next().is_none());
    if empty(base) {
        return base.to_path_buf();
    }
    (2..)
        .map(|i| {
            let mut name = base.as_os_str().to_owned();
            name.push(format!("-{i}"));
            PathBuf::from(name)
        })
        .find(|p| empty(p))
        .expect("an unbounded range always finds a free name")
}

fn tools(cfg: &ScanBench, path: &str, warnings: &mut Vec<String>) -> Vec<Tool> {
    let gx = cfg.gx.to_string_lossy().into_owned();
    let gx_version = env::version(&cfg.gx, "--version").unwrap_or_else(|| "gx".into());
    let mut out = vec![Tool {
        name: "gx-sync",
        version: gx_version.clone(),
        argv: vec![gx.clone(), "scan".into(), path.into()],
    }];
    if cfg.uring {
        out.push(Tool {
            name: "gx-uring",
            version: gx_version,
            argv: vec![
                gx,
                "scan".into(),
                path.into(),
                "--statx".into(),
                "uring".into(),
            ],
        });
    } else {
        warnings.push(
            "gx se compiló sin la feature `uring`: el A/B de statx queda sin la variante B \
             (compilar con `--features uring`)"
                .into(),
        );
    }
    match env::find_in_path("dua") {
        Some(dua) => {
            let help = env::output_of(&dua, &["aggregate", "--help"]).unwrap_or_default();
            if help.contains("Aggregate the consumed space") {
                out.push(Tool {
                    name: "dua",
                    version: env::version(&dua, "--version").unwrap_or_default(),
                    argv: vec![
                        dua.to_string_lossy().into(),
                        "aggregate".into(),
                        path.into(),
                    ],
                });
            } else {
                warnings.push("dua no tiene el subcomando `aggregate`; se omite".into());
            }
        }
        None => warnings.push("dua no está instalado; se omite".into()),
    }
    match env::find_in_path("gdu") {
        Some(gdu) => {
            let help = env::output_of(&gdu, &["--help"]).unwrap_or_default();
            let flags = ["--non-interactive", "--no-progress", "--summarize"];
            if flags.iter().all(|f| help.contains(f)) {
                out.push(Tool {
                    name: "gdu",
                    version: env::version(&gdu, "--version").unwrap_or_default(),
                    argv: std::iter::once(gdu.to_string_lossy().into_owned())
                        .chain(flags.iter().map(|&f| f.to_owned()))
                        .chain(std::iter::once(path.to_owned()))
                        .collect(),
                });
            } else {
                warnings.push(format!("gdu no acepta {flags:?} según su --help; se omite"));
            }
        }
        None => warnings.push("gdu no está instalado; se omite (`sudo pacman -S gdu`)".into()),
    }
    match env::find_in_path("du") {
        Some(du) => out.push(Tool {
            name: "du",
            version: env::version(&du, "--version").unwrap_or_default(),
            argv: vec![
                du.to_string_lossy().into(),
                "-s".into(),
                "-x".into(),
                path.into(),
            ],
        }),
        None => warnings.push("du no está instalado; se omite".into()),
    }
    out
}

fn hyperfine_args(tools: &[Tool], warmup: u32, runs: u32, json: &Path) -> Vec<String> {
    let mut args = vec![
        "-N".to_owned(),
        "--warmup".into(),
        warmup.to_string(),
        "--runs".into(),
        runs.to_string(),
        "--export-json".into(),
        json.to_string_lossy().into(),
    ];
    for t in tools {
        args.push("-n".into());
        args.push(t.name.into());
        args.push(command_line(&t.argv));
    }
    args
}

/// The cold-cache command: hyperfine through a shell, dropping the page
/// cache before every run (needs sudo).
fn cold_command(hyperfine: &str, tools: &[Tool], runs: u32, json: &Path) -> String {
    let mut s = format!(
        "{} --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' --runs {runs} --export-json {}",
        quote(hyperfine),
        quote(&json.to_string_lossy())
    );
    for t in tools {
        let _ = write!(
            s,
            " \\\n    -n {} {}",
            t.name,
            quote(&command_line(&t.argv))
        );
    }
    s
}

fn run_quiet(argv: &[String]) -> Result<std::process::ExitStatus, BenchError> {
    Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| BenchError::Io(argv[0].clone(), e))
}

fn in_process(path: &Path, statx: StatxBackend, runs: u32) -> Result<InProcess, BenchError> {
    let opts = ScanOptions {
        statx,
        ..ScanOptions::default()
    };
    // One warm-up scan, then the measured ones.
    gx_scan::scan_with_stats(path, &opts, &NoProgress)?;
    let mut r = InProcess {
        label: match statx {
            StatxBackend::Sync => "gx-sync",
            StatxBackend::Uring => "gx-uring",
        },
        nodes: 0,
        first_10k: Vec::new(),
        total: Vec::new(),
        bytes_per_node: 0.0,
        columns_and_names_per_node: 0.0,
        uring_threads: 0,
        uring_error: None,
    };
    for _ in 0..runs {
        let (arena, stats) = gx_scan::scan_with_stats(path, &opts, &NoProgress)?;
        let mem = arena.memory_usage();
        let nodes = stats.nodes.max(1) as f64;
        r.nodes = stats.nodes;
        r.bytes_per_node = mem.total() as f64 / nodes;
        r.columns_and_names_per_node = (mem.hot + mem.cold + mem.names) as f64 / nodes;
        r.total.push(stats.elapsed.as_secs_f64());
        if let Some(t) = stats.time_to_10k {
            r.first_10k.push(t.as_secs_f64());
        }
        r.uring_threads = stats.uring_threads;
        r.uring_error = stats.uring_error;
    }
    Ok(r)
}

fn ms(s: f64) -> String {
    if s.is_nan() {
        "—".into()
    } else {
        format!("{:.1} ms", s * 1e3)
    }
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or_else(|| "—".into(), |v| v.to_string())
}

/// Runs the benchmark and returns the results directory.
pub fn run(cfg: &ScanBench) -> Result<PathBuf, BenchError> {
    let path = fs::canonicalize(&cfg.path)
        .map_err(|e| BenchError::Io(cfg.path.display().to_string(), e))?;
    if !path.is_dir() {
        return Err(BenchError::NotADirectory(path));
    }
    let path_s = path.to_string_lossy().into_owned();
    let dir = first_existing_free(&cfg.out.join(&cfg.machine).join(env::today()));
    let mut warnings = Vec::new();
    let tools = tools(cfg, &path_s, &mut warnings);
    let hyperfine = env::find_in_path("hyperfine");
    let time = Path::new("/usr/bin/time");
    let time_ok = env::output_of(time, &["--version"]).is_some_and(|v| v.contains("GNU"));
    let strace = env::find_in_path("strace");
    if hyperfine.is_none() {
        warnings.push("hyperfine no está instalado: no hay tiempos de escaneo".into());
    }
    if !time_ok {
        warnings.push(
            "GNU time (/usr/bin/time) no está instalado: no hay RAM ni CPU (`sudo pacman -S time`)"
                .into(),
        );
    }
    if strace.is_none() {
        warnings.push(
            "strace no está instalado: no hay conteo de syscalls (`sudo pacman -S strace`)".into(),
        );
    }
    match env::governor() {
        Some(g) if g != "performance" => warnings.push(format!(
            "el governor de CPU es `{g}`, no `performance` (F0-validacion.md §V2)"
        )),
        _ => {}
    }
    if env::on_ac_power() == Some(false) {
        warnings.push("el equipo no está conectado a la corriente".into());
    }
    let hyperfine_s = hyperfine
        .as_ref()
        .map_or_else(|| "hyperfine".into(), |h| h.to_string_lossy().into_owned());
    let warm_json = dir.join("scan-caliente.json");
    let cold_json = dir.join("scan-fria.json");

    for w in &warnings {
        eprintln!("AVISO: {w}");
    }
    if cfg.print_only {
        println!("# Caché caliente");
        println!(
            "{} {}",
            quote(&hyperfine_s),
            hyperfine_args(&tools, cfg.warmup, cfg.runs, &warm_json)
                .iter()
                .map(|a| quote(a))
                .collect::<Vec<_>>()
                .join(" ")
        );
        println!("\n# Caché fría (requiere sudo)");
        println!(
            "{}",
            cold_command(&hyperfine_s, &tools, cfg.runs, &cold_json)
        );
        return Ok(dir);
    }

    fs::create_dir_all(&dir).map_err(|e| BenchError::Io(dir.display().to_string(), e))?;
    let write = |name: &str, text: &str| {
        let p = dir.join(name);
        fs::write(&p, text).map_err(|e| BenchError::Io(p.display().to_string(), e))
    };

    // 1. Warm-cache times.
    let mut timings: Vec<Timing> = Vec::new();
    if let Some(h) = &hyperfine {
        let status = Command::new(h)
            .args(hyperfine_args(&tools, cfg.warmup, cfg.runs, &warm_json))
            .status()
            .map_err(|e| BenchError::Io(hyperfine_s.clone(), e))?;
        if !status.success() {
            return Err(BenchError::Failed(format!(
                "hyperfine terminó con {status}"
            )));
        }
        let text = fs::read_to_string(&warm_json)
            .map_err(|e| BenchError::Io(warm_json.display().to_string(), e))?;
        timings =
            hyperfine_timings(&json::parse(&text)?).map_err(|e| BenchError::Failed(e.into()))?;
    }

    // 2. Resources and syscalls, one separate run each.
    let mut resources = Vec::new();
    let mut syscalls = Vec::new();
    for t in &tools {
        if time_ok {
            let out = dir.join(format!("time-{}.txt", t.name));
            let mut argv = vec![
                time.to_string_lossy().into_owned(),
                "-v".into(),
                "-o".into(),
                out.to_string_lossy().into_owned(),
            ];
            argv.extend(t.argv.iter().cloned());
            run_quiet(&argv)?;
            let text = fs::read_to_string(&out).unwrap_or_default();
            resources.push((t.name, parse_time_v(&text)));
        }
        if let Some(s) = &strace {
            let out = dir.join(format!("strace-{}.txt", t.name));
            let mut argv = vec![
                s.to_string_lossy().into_owned(),
                "-c".into(),
                "-f".into(),
                "-o".into(),
                out.to_string_lossy().into_owned(),
            ];
            argv.extend(t.argv.iter().cloned());
            run_quiet(&argv)?;
            let text = fs::read_to_string(&out).unwrap_or_default();
            syscalls.push((t.name, parse_strace_total(&text)));
        }
    }

    // 3. The scanner in-process: first 10 000 nodes and bytes per node.
    let mut inproc = vec![in_process(&path, StatxBackend::Sync, cfg.runs)?];
    if cfg.uring {
        inproc.push(in_process(&path, StatxBackend::Uring, cfg.runs)?);
    }

    // 4. Summary.
    let mut md = String::new();
    let _ = writeln!(md, "# Escaneo de `{path_s}`\n");
    let _ = writeln!(md, "## Entorno\n");
    let _ = writeln!(md, "| Dato | Valor |\n|---|---|");
    let _ = writeln!(md, "| Equipo | {} |", cfg.machine);
    let _ = writeln!(md, "| Fecha | {} |", env::today());
    let _ = writeln!(md, "| Commit | {} |", opt(env::git_commit()));
    let _ = writeln!(md, "| Kernel | {} |", env::kernel());
    let _ = writeln!(md, "| Governor | {} |", opt(env::governor()));
    let _ = writeln!(
        md,
        "| Corriente | {} |",
        env::on_ac_power().map_or("—", |on| if on { "conectado" } else { "batería" })
    );
    let _ = writeln!(
        md,
        "| hyperfine | {} |",
        opt(hyperfine
            .as_deref()
            .and_then(|h| env::version(h, "--version")))
    );
    let _ = writeln!(
        md,
        "| Corridas | {} medidas + {} de calentamiento |",
        cfg.runs, cfg.warmup
    );
    if !warnings.is_empty() {
        let _ = writeln!(md, "\n**Avisos:**\n");
        for w in &warnings {
            let _ = writeln!(md, "- {w}");
        }
    }

    let _ = writeln!(md, "\n## Herramientas\n");
    let _ = writeln!(
        md,
        "| Herramienta | Versión | Comando exacto |\n|---|---|---|"
    );
    for t in &tools {
        let _ = writeln!(
            md,
            "| {} | {} | `{}` |",
            t.name,
            t.version,
            command_line(&t.argv)
        );
    }

    let _ = writeln!(md, "\n## Escaneo con caché caliente\n");
    let _ = writeln!(
        md,
        "p95 por rango más cercano: con 10 corridas es el máximo.\n"
    );
    let _ = writeln!(
        md,
        "| Herramienta | Mediana | p95 | Media | CPU usuario (media) | CPU sistema (media) |\n|---|---|---|---|---|---|"
    );
    for t in &timings {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} |",
            t.name,
            ms(t.median),
            ms(t.p95),
            ms(t.mean),
            ms(t.user),
            ms(t.system)
        );
    }

    let _ = writeln!(md, "\n## gx en proceso ({} corridas)\n", cfg.runs);
    let _ = writeln!(
        md,
        "| Variante | Nodos | Hasta 10.000 nodos (mediana / p95) | Total (mediana / p95) | B/nodo total | B/nodo columnas + nombres | Hilos con io_uring |\n|---|---|---|---|---|---|---|"
    );
    for r in &inproc {
        let _ = writeln!(
            md,
            "| {} | {} | {} / {} | {} / {} | {:.1} | {:.1} | {}{} |",
            r.label,
            r.nodes,
            ms(median(&r.first_10k)),
            ms(percentile(&r.first_10k, 95.0)),
            ms(median(&r.total)),
            ms(percentile(&r.total, 95.0)),
            r.bytes_per_node,
            r.columns_and_names_per_node,
            r.uring_threads,
            r.uring_error
                .map_or_else(String::new, |e| format!(" (caída a sync, errno {e})"))
        );
    }

    let _ = writeln!(md, "\n## Recursos (una corrida, caché caliente)\n");
    let _ = writeln!(
        md,
        "| Herramienta | RAM máxima | CPU usuario | CPU sistema | Syscalls |\n|---|---|---|---|---|"
    );
    for t in &tools {
        let r = resources
            .iter()
            .find(|(n, _)| *n == t.name)
            .map(|(_, r)| *r)
            .unwrap_or_default();
        let calls = syscalls
            .iter()
            .find(|(n, _)| *n == t.name)
            .and_then(|(_, c)| *c);
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} |",
            t.name,
            r.max_rss_kib
                .map_or_else(|| "—".into(), |k| format!("{:.1} MiB", k as f64 / 1024.0)),
            r.user_s.map_or_else(|| "—".into(), |s| format!("{s:.2} s")),
            r.system_s
                .map_or_else(|| "—".into(), |s| format!("{s:.2} s")),
            opt(calls)
        );
    }
    let _ = writeln!(
        md,
        "\nCon io_uring, cada `statx` del lote no es una syscall: strace solo ve los `io_uring_enter`."
    );

    let _ = writeln!(md, "\n## Caché fría (ejecutar a mano, requiere sudo)\n");
    let _ = writeln!(
        md,
        "```sh\n{}\n```\n",
        cold_command(&hyperfine_s, &tools, cfg.runs, &cold_json)
    );
    let _ = writeln!(
        md,
        "Resumen del JSON: `{} bench report {}`.\n",
        quote(&cfg.gx.to_string_lossy()),
        quote(&cold_json.to_string_lossy())
    );
    let _ = writeln!(
        md,
        "Tiempo hasta 10.000 nodos con caché fría (repetir y anotar la mediana):\n\n```sh\nsync; echo 3 | sudo tee /proc/sys/vm/drop_caches; {} scan {}\n```",
        quote(&cfg.gx.to_string_lossy()),
        quote(&path_s)
    );
    write("resumen.md", &md)?;
    println!("{md}");
    println!("Resultados en {}", dir.display());
    Ok(dir)
}

/// Prints median and p95 of every command of hyperfine exports.
pub fn report(files: &[PathBuf]) -> Result<(), BenchError> {
    for f in files {
        let text = fs::read_to_string(f).map_err(|e| BenchError::Io(f.display().to_string(), e))?;
        let timings =
            hyperfine_timings(&json::parse(&text)?).map_err(|e| BenchError::Failed(e.into()))?;
        println!("{}\n", f.display());
        println!("| Comando | Corridas | Mediana | p95 | Media |\n|---|---|---|---|---|");
        for t in timings {
            println!(
                "| {} | {} | {} | {} | {} |",
                t.name,
                t.runs,
                ms(t.median),
                ms(t.p95),
                ms(t.mean)
            );
        }
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gnu_time() {
        let text = "\tCommand being timed: \"gx scan /x\"\n\tUser time (seconds): 0.52\n\tSystem time (seconds): 1.25\n\tMaximum resident set size (kbytes): 81234\n";
        let r = parse_time_v(text);
        assert_eq!(r.max_rss_kib, Some(81234));
        assert_eq!(r.user_s, Some(0.52));
        assert_eq!(r.system_s, Some(1.25));
    }

    #[test]
    fn parses_strace_total() {
        let with_errors = "% time     seconds  usecs/call     calls    errors syscall\n------ ----------- ----------- --------- --------- ----------------\n 60.00    0.000600           1       500        10 statx\n------ ----------- ----------- --------- --------- ----------------\n100.00    0.001000           1       812        12 total\n";
        assert_eq!(parse_strace_total(with_errors), Some(812));
        let without = "100.00    0.001000           1       812 total\n";
        assert_eq!(parse_strace_total(without), Some(812));
        assert_eq!(parse_strace_total("nada"), None);
    }
}
