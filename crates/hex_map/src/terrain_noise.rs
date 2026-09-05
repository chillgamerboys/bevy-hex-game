//! Shared integer terrain noise used by voxel materials and their visual review.

use hex_core::HexCoord;
use xxhash_rust::xxh3::xxh3_64_with_seed;

/// Returns a bounded, spatially coherent level offset with platform-stable integer interpolation.
pub(crate) fn coherent_level_offset(
    seed: u64,
    domain: &[u8],
    coord: HexCoord,
    correlation_hexes: u16,
    amplitude: i32,
) -> i32 {
    if correlation_hexes == 0 || amplitude == 0 {
        return 0;
    }
    let cell = i32::from(correlation_hexes);
    let x0 = coord.x().div_euclid(cell);
    let y0 = coord.y().div_euclid(cell);
    let x_fraction = coord.x().rem_euclid(cell);
    let y_fraction = coord.y().rem_euclid(cell);
    let north_west = coherent_corner(seed, domain, x0, y0);
    let north_east = coherent_corner(seed, domain, x0.saturating_add(1), y0);
    let south_west = coherent_corner(seed, domain, x0, y0.saturating_add(1));
    let south_east = coherent_corner(seed, domain, x0.saturating_add(1), y0.saturating_add(1));
    let north = integer_lerp(north_west, north_east, x_fraction, cell);
    let south = integer_lerp(south_west, south_east, x_fraction, cell);
    let noise = integer_lerp(north, south, y_fraction, cell);
    rounded_ratio(
        i64::from(noise).saturating_mul(i64::from(amplitude)),
        32_768,
    )
}

fn coherent_corner(seed: u64, domain: &[u8], x: i32, y: i32) -> i32 {
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(8));
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    let sample = xxh3_64_with_seed(&bytes, seed);
    let high = u16::try_from(sample >> 48).unwrap_or(0);
    i32::from(high).saturating_sub(32_768)
}

fn integer_lerp(first: i32, second: i32, numerator: i32, denominator: i32) -> i32 {
    if denominator <= 0 {
        return first;
    }
    let first_weight = denominator.saturating_sub(numerator);
    let weighted = i64::from(first)
        .saturating_mul(i64::from(first_weight))
        .saturating_add(i64::from(second).saturating_mul(i64::from(numerator)));
    let value = weighted / i64::from(denominator);
    i32::try_from(value).unwrap_or_else(|_| {
        if value.is_negative() {
            i32::MIN
        } else {
            i32::MAX
        }
    })
}

fn rounded_ratio(numerator: i64, denominator: i64) -> i32 {
    if denominator <= 0 {
        return 0;
    }
    let half = denominator / 2;
    let rounded = if numerator.is_negative() {
        numerator.saturating_sub(half) / denominator
    } else {
        numerator.saturating_add(half) / denominator
    };
    i32::try_from(rounded).unwrap_or_else(|_| {
        if rounded.is_negative() {
            i32::MIN
        } else {
            i32::MAX
        }
    })
}

