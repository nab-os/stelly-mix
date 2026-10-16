//! A steady beat grid through tracked beats: what a DJ deck syncs to.
//!
//! The tracker's beats are frame-accurate and follow every wobble. Most
//! electronic music is made on a grid, so a line fitted through them gives
//! the tempo to a hundredth of a BPM and the phase to a millisecond. How far
//! the beats stray from that line says whether the track has a grid at all:
//! a live drummer drifts, and syncing such a track wants the beats, not this.

use serde::Serialize;

/// How close a beat must be to the grid to count as on it, in seconds. The
/// tracker alone is good to about half a frame, ~5ms.
pub const ON_GRID: f64 = 0.020;
/// Share of beats on the grid for a track to count as made on one. Not all:
/// the tracker slips for a bar or two in a beatless breakdown.
pub const STEADY_SHARE: f64 = 0.8;

#[derive(Debug, Clone, Serialize)]
pub struct Grid {
    pub bpm: f64,
    /// Seconds to the first grid line at or after the start of the audio.
    pub first_beat: f64,
    /// RMS distance of the tracked beats from the grid, in milliseconds.
    pub rms_ms: f64,
    /// Share of the tracked beats within `ON_GRID` of it.
    pub on_grid: f64,
    pub steady: bool,
}

impl Grid {
    pub fn period(&self) -> f64 {
        60.0 / self.bpm
    }
}

/// Least squares of beat time against beat number. Numbers come from the
/// gaps rather than the index, so a beat the tracker skipped does not shear
/// the whole line, and outliers are dropped before a refit.
pub fn fit(beats: &[f64]) -> Option<Grid> {
    if beats.len() < 8 {
        return None;
    }
    let mut gaps: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(f64::total_cmp);
    let rough = gaps[gaps.len() / 2];

    // Each beat numbered from the gap to the one before it, which a rough
    // period gets right; counting from the first beat would let its error
    // pile up over a few hundred beats.
    let mut number = 0.0;
    let numbered: Vec<(f64, f64)> = beats
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if i > 0 {
                number += ((b - beats[i - 1]) / rough).round().max(1.0);
            }
            (number, b)
        })
        .collect();
    let (origin, period) = line(&numbered)?;

    let residuals: Vec<f64> = numbered.iter().map(|&(i, b)| b - (origin + period * i)).collect();
    let mut spread: Vec<f64> = residuals.iter().map(|r| r.abs()).collect();
    spread.sort_by(f64::total_cmp);
    let cutoff = (3.0 * 1.4826 * spread[spread.len() / 2]).max(0.002);
    let kept: Vec<(f64, f64)> = numbered
        .iter()
        .zip(&residuals)
        .filter(|(_, r)| r.abs() <= cutoff)
        .map(|(p, _)| *p)
        .collect();
    let (origin, period) = line(&kept).unwrap_or((origin, period));

    // Measured over every beat, outliers included.
    let residuals: Vec<f64> = numbered.iter().map(|&(i, b)| b - (origin + period * i)).collect();
    let n = residuals.len() as f64;
    let rms_ms = (residuals.iter().map(|r| r * r).sum::<f64>() / n).sqrt() * 1e3;
    let on_grid = residuals.iter().filter(|r| r.abs() <= ON_GRID).count() as f64 / n;

    Some(Grid {
        bpm: 60.0 / period,
        first_beat: origin.rem_euclid(period),
        rms_ms,
        on_grid,
        steady: on_grid >= STEADY_SHARE,
    })
}

/// Intercept and slope of y on x.
fn line(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    if n < 2.0 {
        return None;
    }
    let (mx, my) = (
        points.iter().map(|p| p.0).sum::<f64>() / n,
        points.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    if sxx <= 0.0 {
        return None;
    }
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let slope = sxy / sxx;
    Some((my - slope * mx, slope))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jittered_beats_with_a_gap_fit_the_grid() {
        let (bpm, offset) = (127.5, 0.42);
        let period = 60.0 / bpm;
        let beats: Vec<f64> = (0..200)
            .filter(|i| !(80..84).contains(i))
            .map(|i| offset + period * i as f64 + if i % 2 == 0 { 0.004 } else { -0.004 })
            .collect();
        let grid = fit(&beats).unwrap();
        assert!((grid.bpm - bpm).abs() < 0.01, "{}", grid.bpm);
        assert!((grid.first_beat - offset).abs() < 0.002, "{}", grid.first_beat);
        assert!(grid.steady);
    }

    #[test]
    fn a_drifting_tempo_is_not_steady() {
        let mut t = 0.0;
        let beats: Vec<f64> = (0..300)
            .map(|i| {
                t += 60.0 / (120.0 + 8.0 * (i as f64 / 40.0).sin());
                t
            })
            .collect();
        assert!(!fit(&beats).unwrap().steady);
    }
}
