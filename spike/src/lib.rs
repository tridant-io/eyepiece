//! shared helpers for the phase 0 gate programs. throwaway.

pub mod ff;
pub mod hap;
#[cfg(windows)]
pub mod d3d;
pub mod dec;
#[cfg(windows)]
pub mod win;

use std::time::Instant;

/// prints the verdict line every gate ends with and returns the process exit code.
pub fn verdict(pass: bool, why: &str) -> i32 {
    println!("{} {}", if pass { "PASS" } else { "FAIL" }, why);
    if pass { 0 } else { 1 }
}

pub fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// p50 and p95 of a set of samples.
pub fn p50_p95(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.is_empty() {
        return (0.0, 0.0);
    }
    let at = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    (at(0.5), at(0.95))
}
