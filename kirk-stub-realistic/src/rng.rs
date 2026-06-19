//! Seeded RNG factory aligned with `rand_xoshiro::Xoshiro256StarStar`.

use rand::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;

/// Construct a deterministic Xoshiro256** RNG from a u64 seed.
pub fn seeded(seed: u64) -> Xoshiro256StarStar {
    Xoshiro256StarStar::seed_from_u64(seed)
}
