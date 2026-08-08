//! Shared PNG capture plumbing for the automation features.
//!
//! Both `map-review` (single deterministic gameplay capture) and `visual-walk`
//! (scripted multi-screen walk) screenshot the renderer and persist the frame
//! atomically. The plumbing lives here once; *policy* stays with each caller:
//! review demands its exact 1920x1080 target and rejects frames without full
//! visual coverage, while the walk captures an explicit logical-size/device-scale
//! image target. The walk's live UI-tree oracle is its structural gate; pixel
//! coverage remains supporting evidence because a valid menu is mostly flat
//! background and would fail review's terrain-shaped thresholds.

use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use bevy::prelude::*;

const VARIATION_GRID_COLUMNS: usize = 8;
const VARIATION_GRID_ROWS: usize = 4;
const MIN_VARIANT_PIXEL_PERCENT: usize = 5;
const MIN_VARIED_REGIONS: usize = 8;

/// What a persisted frame looked like, for the caller's policy to judge.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CaptureStats {
    /// The brightest channel value anywhere in the frame; `<= 8` means the
    /// frame is effectively black.
    pub(crate) brightest: u8,
    /// Whether the frame passes the review-grade variation thresholds.
    pub(crate) has_coverage: bool,
}

/// Converts a captured frame to RGB, writes it atomically, and reports stats.
///
/// The PNG is installed even when its content looks empty — a rejected frame
/// on disk is inspectable, a refused write is not. Callers decide whether the
/// returned stats are a failure.
pub(crate) fn write_png(image: &Image, path: &Path) -> Result<CaptureStats, String> {
    let dynamic = image
        .clone()
        .try_into_dynamic()
        .map_err(|error| format!("cannot convert renderer output: {error}"))?;
    let rgb = dynamic.to_rgb8();
    let analysis = analyze_coverage(
        rgb.as_raw(),
        usize::try_from(rgb.width()).unwrap_or_default(),
        usize::try_from(rgb.height()).unwrap_or_default(),
    );

    prepare_capture_path(path)
        .map_err(|error| format!("cannot prepare staged screenshot output: {error}"))?;
    let temporary = temporary_capture_path(path)
        .map_err(|error| format!("cannot prepare staged screenshot output: {error}"))?;
    if let Err(error) = rgb.save(&temporary) {
        let _cleanup = fs::remove_file(&temporary);
        return Err(format!("cannot write temporary PNG: {error}"));
    }
    if let Err(error) = install_capture(&temporary, path) {
        let _cleanup = fs::remove_file(&temporary);
        return Err(format!("cannot install PNG: {error}"));
    }
    Ok(CaptureStats {
        brightest: analysis.brightest,
        has_coverage: analysis.has_coverage,
    })
}

/// Creates the output directory and clears any stale temporary file.
pub(crate) fn prepare_capture_path(path: &Path) -> std::io::Result<()> {
    if path.file_name().is_none() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "capture path must name a file",
        ));
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    match fs::remove_file(temporary_capture_path(path)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Installs a complete temporary PNG, including over an existing Windows file.
///
/// `std::fs::rename` replaces files on Unix but rejects an existing destination on
/// Windows. Removing the old destination only after the new PNG is fully written
/// keeps the capture path portable without exposing a partially encoded image.
pub(crate) fn install_capture(temporary: &Path, path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::rename(temporary, path)
}

/// The hidden sibling path a capture is staged at before its atomic install.
pub(crate) fn temporary_capture_path(path: &Path) -> std::io::Result<PathBuf> {
    let Some(file_name) = path.file_name() else {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "capture path must name a file",
        ));
    };
    let mut temporary_name = OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".{}.tmp.png", std::process::id()));
    Ok(path.with_file_name(temporary_name))
}

struct CoverageAnalysis {
    brightest: u8,
    has_coverage: bool,
}

/// Whether a frame passes the review-grade variation thresholds.
///
/// Production callers go through [`write_png`]'s stats; the tests exercise the
/// analyzer directly against synthetic frames.
#[cfg(all(test, feature = "map-review"))]
pub(crate) fn has_visual_coverage(bytes: &[u8], width: usize, height: usize) -> bool {
    analyze_coverage(bytes, width, height).has_coverage
}

fn analyze_coverage(bytes: &[u8], width: usize, height: usize) -> CoverageAnalysis {
    let rejected = CoverageAnalysis {
        brightest: 0,
        has_coverage: false,
    };
    let Some(pixel_count) = width.checked_mul(height) else {
        return rejected;
    };
    if pixel_count == 0 || bytes.len() != pixel_count.saturating_mul(3) {
        return rejected;
    }

    let region_count = VARIATION_GRID_COLUMNS.saturating_mul(VARIATION_GRID_ROWS);
    let mut region_minimums = vec![[u8::MAX; 3]; region_count];
    let mut region_maximums = vec![[u8::MIN; 3]; region_count];
    let mut histogram = vec![0_usize; 16 * 16 * 16];
    let mut brightest = u8::MIN;

    for (index, pixel) in bytes.chunks_exact(3).enumerate() {
        let &[red, green, blue] = pixel else {
            return rejected;
        };
        brightest = brightest.max(red).max(green).max(blue);
        let bin =
            usize::from(red >> 4) * 16 * 16 + usize::from(green >> 4) * 16 + usize::from(blue >> 4);
        let Some(count) = histogram.get_mut(bin) else {
            return rejected;
        };
        *count = count.saturating_add(1);

        let x = index % width;
        let y = index / width;
        let region_x = x.saturating_mul(VARIATION_GRID_COLUMNS) / width;
        let region_y = y.saturating_mul(VARIATION_GRID_ROWS) / height;
        let region = region_y
            .saturating_mul(VARIATION_GRID_COLUMNS)
            .saturating_add(region_x)
            .min(region_count.saturating_sub(1));
        let Some((minimums, maximums)) = region_minimums
            .get_mut(region)
            .zip(region_maximums.get_mut(region))
        else {
            return rejected;
        };
        for ((minimum, maximum), value) in minimums
            .iter_mut()
            .zip(maximums.iter_mut())
            .zip([red, green, blue])
        {
            *minimum = (*minimum).min(value);
            *maximum = (*maximum).max(value);
        }
    }

    let dominant = histogram.into_iter().max().unwrap_or(pixel_count);
    let variant_pixels = pixel_count.saturating_sub(dominant);
    let varied_regions = region_minimums
        .iter()
        .zip(region_maximums.iter())
        .filter(|(minimum, maximum)| {
            minimum
                .iter()
                .zip(maximum.iter())
                .any(|(low, high)| high.abs_diff(*low) > 12)
        })
        .count();

    let has_coverage = brightest > 8
        && variant_pixels.saturating_mul(100)
            >= pixel_count.saturating_mul(MIN_VARIANT_PIXEL_PERCENT)
        && varied_regions >= MIN_VARIED_REGIONS;
    CoverageAnalysis {
        brightest,
        has_coverage,
    }
}
