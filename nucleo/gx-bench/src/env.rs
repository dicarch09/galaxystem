// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The measurement environment: external tools, machine and date.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Finds an executable in `PATH`.
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Combined stdout and stderr of a command, if it could be run.
pub fn output_of(program: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

/// First non-empty line of `program <arg>`.
pub fn version(program: &Path, arg: &str) -> Option<String> {
    output_of(program, &[arg])?
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
}

fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Host name.
pub fn hostname() -> String {
    read_trimmed("/proc/sys/kernel/hostname").unwrap_or_else(|| "equipo".into())
}

/// Kernel release.
pub fn kernel() -> String {
    read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_default()
}

/// CPU frequency governor of CPU 0.
pub fn governor() -> Option<String> {
    read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
}

/// Whether a mains power supply is online (`None` if there is none, as on
/// most desktops).
pub fn on_ac_power() -> Option<bool> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut found = None;
    for e in entries.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Mains") {
            let online = std::fs::read_to_string(p.join("online")).is_ok_and(|o| o.trim() == "1");
            found = Some(found.unwrap_or(false) || online);
        }
    }
    found
}

/// Short commit of the repository in the current directory, with `-dirty`
/// if it has uncommitted changes.
pub fn git_commit() -> Option<String> {
    let git = find_in_path("git")?;
    let commit = output_of(&git, &["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_owned();
    if commit.is_empty() || commit.contains(' ') {
        return None;
    }
    let dirty = Command::new(&git)
        .args(["diff", "--quiet", "HEAD"])
        .status()
        .is_ok_and(|s| !s.success());
    Some(if dirty { commit + "-dirty" } else { commit })
}

/// Today's local date as `YYYY-MM-DD` (from `date`, or UTC as fallback).
pub fn today() -> String {
    if let Some(d) = find_in_path("date")
        .and_then(|date| output_of(&date, &["+%F"]))
        .map(|s| s.trim().to_owned())
        .filter(|s| s.len() == 10)
    {
        return d;
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Gregorian date of a day count since 1970-01-01 (H. Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Quotes an argument for a POSIX shell (and for hyperfine's `-N`
/// splitting) when needed.
pub fn quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b));
    if safe {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// A command line, quoted.
pub fn command_line(argv: &[String]) -> String {
    argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_766), (2026, 11, 9));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("/a/b-c.txt"), "/a/b-c.txt");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
    }
}
