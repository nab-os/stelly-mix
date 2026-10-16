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

/// Beats in the stretch the grid is first fitted to.
const SEED_BEATS: usize = 32;

/// The grid most of the beats agree on. Fitted first to the steadiest
/// stretch of `SEED_BEATS`, then extended: every beat numbered by the grid
/// line nearest it, the line refitted through those on it, until that stops
/// changing. A breakdown the tracker wandered through is then just beats off
/// the grid, rather than a miscount that shears everything after it.
pub fn fit(beats: &[f64]) -> Option<Grid> {
    if beats.len() < 8 {
        return None;
    }
    let (mut origin, mut period) = seed(beats)?;

    // The seed's period is good to a fraction of a millisecond, which over a
    // few hundred beats can still add up to more than `ON_GRID`: the first
    // pass takes anything within a quarter beat, the rest only what is on it.
    let mut tolerance = period / 4.0;
    let mut kept = 0;
    for _ in 0..10 {
        let on: Vec<(f64, f64)> = beats
            .iter()
            .map(|&b| (((b - origin) / period).round(), b))
            .filter(|&(n, b)| (b - (origin + period * n)).abs() <= tolerance)
            .collect();
        if on.len() < 8 {
            break;
        }
        (origin, period) = line(&on)?;
        if tolerance == ON_GRID && on.len() == kept {
            break;
        }
        kept = on.len();
        tolerance = ON_GRID;
    }

    let residuals: Vec<f64> = beats
        .iter()
        .map(|&b| {
            let n = ((b - origin) / period).round();
            b - (origin + period * n)
        })
        .collect();
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

/// The line through the stretch of beats that sits on one best. Within a
/// stretch, beats are numbered from the gaps between them, which a skipped
/// beat cannot fool.
fn seed(beats: &[f64]) -> Option<(f64, f64)> {
    let width = SEED_BEATS.min(beats.len());
    let mut best: Option<((f64, f64), usize)> = None;
    for start in (0..=beats.len() - width).step_by((width / 4).max(1)) {
        let stretch = &beats[start..start + width];
        let mut gaps: Vec<f64> = stretch.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(f64::total_cmp);
        let rough = gaps[gaps.len() / 2];
        let mut number = 0.0;
        let numbered: Vec<(f64, f64)> = stretch
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                if i > 0 {
                    number += ((b - stretch[i - 1]) / rough).round().max(1.0);
                }
                (number, b)
            })
            .collect();
        let Some((origin, period)) = line(&numbered) else { continue };
        let on = numbered
            .iter()
            .filter(|&&(n, b)| (b - (origin + period * n)).abs() <= ON_GRID)
            .count();
        if best.is_none_or(|(_, most)| on > most) {
            best = Some(((origin, period), on));
        }
    }
    best.map(|(line, _)| line)
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

    /// A breakdown with no kick: the tracker wanders off at its own spacing,
    /// then the drop brings it back onto the grid. Twice.
    #[test]
    fn breakdowns_do_not_shear_the_grid() {
        let (bpm, offset) = (155.0, 0.3);
        let period = 60.0 / bpm;
        let on = |i: usize| offset + period * i as f64;
        let mut beats: Vec<f64> = (0..120).map(on).collect();
        beats.extend((0..10).map(|k| on(120) + 0.137 + 1.37 * period * k as f64));
        beats.extend((135..300).map(on));
        beats.extend((0..12).map(|k| on(300) + 0.21 + 1.43 * period * k as f64));
        beats.extend((320..500).map(on));

        let grid = fit(&beats).unwrap();
        assert!((grid.bpm - bpm).abs() < 0.01, "{}", grid.bpm);
        let phase = (grid.first_beat - offset).abs();
        assert!(phase < 0.002, "first beat {}", grid.first_beat);
        assert!(grid.on_grid > 0.95, "{:.0}% on grid", grid.on_grid * 100.0);
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
