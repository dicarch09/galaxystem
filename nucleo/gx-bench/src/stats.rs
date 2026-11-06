// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Medians and percentiles, and the timings of a hyperfine export.

use crate::json::Value;

/// Median of `values` (any order). `NaN` if empty.
pub fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => f64::NAN,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]) / 2.0,
    }
}

/// Percentile `p` (0–100) by the nearest-rank method: the smallest value
/// with at least `p` % of the values at or below it. With 10 runs, p95 is
/// the maximum. `NaN` if empty.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        return f64::NAN;
    }
    let rank = ((p / 100.0) * v.len() as f64).ceil().max(1.0) as usize;
    v[rank.min(v.len()) - 1]
}

/// Timing of one command of a hyperfine export, in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// Command name (`-n`) or command line.
    pub name: String,
    /// Number of measured runs.
    pub runs: usize,
    /// Median wall time.
    pub median: f64,
    /// p95 wall time (nearest rank).
    pub p95: f64,
    /// Mean wall time.
    pub mean: f64,
    /// Mean user CPU time.
    pub user: f64,
    /// Mean system CPU time.
    pub system: f64,
}

/// Extracts the timings of every command in a hyperfine `--export-json`
/// document. Median and p95 are computed from the individual `times`.
pub fn hyperfine_timings(doc: &Value) -> Result<Vec<Timing>, &'static str> {
    let results = doc
        .get("results")
        .and_then(Value::as_array)
        .ok_or("falta el arreglo \"results\"")?;
    results
        .iter()
        .map(|r| {
            let times: Vec<f64> = r
                .get("times")
                .and_then(Value::as_array)
                .ok_or("falta \"times\"")?
                .iter()
                .filter_map(Value::as_f64)
                .collect();
            let num = |k: &str| r.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
            Ok(Timing {
                name: r
                    .get("command")
                    .and_then(Value::as_str)
                    .ok_or("falta \"command\"")?
                    .to_owned(),
                runs: times.len(),
                median: median(&times),
                p95: percentile(&times, 95.0),
                mean: num("mean"),
                user: num("user"),
                system: num("system"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_and_nearest_rank() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        let ten: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(percentile(&ten, 95.0), 10.0);
        assert_eq!(percentile(&ten, 50.0), 5.0);
        let hundred: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&hundred, 95.0), 95.0);
        assert!(median(&[]).is_nan());
    }

    #[test]
    fn timings_from_export() {
        let doc = crate::json::parse(
            r#"{"results":[{"command":"gx","mean":0.2,"user":0.1,"system":0.3,
                "times":[0.1,0.3,0.2]}]}"#,
        )
        .unwrap();
        let t = hyperfine_timings(&doc).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].name, "gx");
        assert_eq!((t[0].runs, t[0].median, t[0].p95), (3, 0.2, 0.3));
        assert!(hyperfine_timings(&crate::json::parse("{}").unwrap()).is_err());
    }
}
