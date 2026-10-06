//! Helpers shared by the unit tests.
use rand::{SeedableRng, rngs::StdRng};

/// Seed used when `TEST_SEED` is not set, so every run tests the same cases.
const DEFAULT_SEED: u64 = 0x5EED_5EED;

/// A random generator for `test`, seeded so that a failure can be replayed.
///
/// The seed is `TEST_SEED` if set (decimal or `0x`-prefixed hex), else [`DEFAULT_SEED`]. It is
/// printed, and `cargo test` shows the output of a failing test, so its seed is always visible.
pub(crate) fn seeded_rng(test: &str) -> StdRng {
    let seed = std::env::var("TEST_SEED").map_or(DEFAULT_SEED, |value| {
        parse_seed(&value).unwrap_or_else(|| panic!("TEST_SEED={value:?} is not a u64"))
    });
    println!("{test}: seed {seed:#x} (rerun with TEST_SEED={seed:#x})");
    StdRng::seed_from_u64(seed)
}

fn parse_seed(value: &str) -> Option<u64> {
    match value.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => value.parse().ok(),
    }
}
