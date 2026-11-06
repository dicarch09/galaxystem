// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Deterministic random helpers and distributions.
//!
//! Implemented here instead of pulling `rand_distr`, which is not in the
//! allowed dependency list (F0-diseno.md §9).

use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::{Rng as _, SeedableRng};

/// Seeded generator used by the planner.
pub struct Rng(Xoshiro256PlusPlus);

impl Rng {
    /// Creates a generator from a 64-bit seed.
    pub fn new(seed: u64) -> Self {
        Rng(Xoshiro256PlusPlus::seed_from_u64(seed))
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `(0, 1]`, safe for `ln`.
    pub fn unit_open(&mut self) -> f64 {
        1.0 - self.unit()
    }

    /// Uniform integer in `[0, n)`; `n` must be positive.
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    /// `true` with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Standard normal sample (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let u1 = self.unit_open();
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (core::f64::consts::TAU * u2).cos()
    }

    /// Log-normal sample with parameters `mu` and `sigma` (natural log).
    pub fn log_normal(&mut self, mu: f64, sigma: f64) -> f64 {
        (mu + sigma * self.normal()).exp()
    }

    /// Pareto sample with scale 1 and shape `alpha`.
    pub fn pareto(&mut self, alpha: f64) -> f64 {
        self.unit_open().powf(-1.0 / alpha)
    }

    /// Picks an element of a non-empty slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Fenwick tree over non-negative weights, for weighted sampling with
/// growing item sets in `O(log n)`.
pub struct Fenwick {
    tree: Vec<f64>,
    len: usize,
    total: f64,
}

impl Fenwick {
    /// Empty tree with room for `capacity` items.
    pub fn with_capacity(capacity: usize) -> Self {
        Fenwick {
            tree: vec![0.0; capacity + 1],
            len: 0,
            total: 0.0,
        }
    }

    /// Appends an item with weight `w` and returns its index.
    pub fn push(&mut self, w: f64) -> usize {
        let i = self.len;
        assert!(i + 1 < self.tree.len(), "Fenwick capacity exceeded");
        self.len += 1;
        self.add(i, w);
        i
    }

    fn add(&mut self, i: usize, w: f64) {
        let mut k = i + 1;
        while k < self.tree.len() {
            self.tree[k] += w;
            k += k & k.wrapping_neg();
        }
        self.total += w;
    }

    /// Index of the item where the cumulative weight first exceeds `u`
    /// (`0 <= u < total`).
    pub fn find(&self, mut u: f64) -> usize {
        let mut pos = 0;
        let mut step = (self.tree.len() - 1).next_power_of_two();
        while step > 0 {
            let next = pos + step;
            if next < self.tree.len() && self.tree[next] <= u {
                pos = next;
                u -= self.tree[next];
            }
            step >>= 1;
        }
        pos.min(self.len.saturating_sub(1))
    }

    /// Samples an index with probability proportional to its weight.
    pub fn sample(&self, rng: &mut Rng) -> usize {
        self.find(rng.unit() * self.total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn ranges() {
        let mut r = Rng::new(1);
        for _ in 0..10_000 {
            assert!(r.below(10) < 10);
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
            assert!(r.pareto(1.2) >= 1.0);
            assert!(r.log_normal(9.0, 3.0) > 0.0);
        }
    }

    #[test]
    fn fenwick_sampling_respects_weights() {
        let mut f = Fenwick::with_capacity(4);
        f.push(1.0);
        f.push(0.0);
        f.push(3.0);
        assert_eq!(f.find(0.5), 0);
        assert_eq!(f.find(1.0), 2);
        assert_eq!(f.find(3.9), 2);
        let mut r = Rng::new(3);
        let mut counts = [0u32; 3];
        for _ in 0..4000 {
            counts[f.sample(&mut r)] += 1;
        }
        assert_eq!(counts[1], 0);
        assert!(counts[2] > counts[0] * 2);
    }
}
