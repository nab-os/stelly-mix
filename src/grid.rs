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
/// A track counts as made on a grid when the stretches of it that keep a
/// steady beat of their own agree with it: at least `STEADY_STRETCHES` of
/// them have half their beats on the global grid. Not a share of all beats:
/// in a long beatless breakdown the tracker wanders, which can leave a
/// gridded hard tech track at 60%, but a wandering stretch has no steady beat
/// of its own and so no say. A drifting live take is steady stretch by
/// stretch, and each stretch disagrees with any one grid.
const STEADY_STRETCHES: f64 = 0.75;

#[derive(Debug, Clone, Serialize)]
pub struct Grid {
    pub bpm: f64,
    /// Seconds to the first grid line at or after the start of the audio.
    pub first_beat: f64,
    /// RMS distance of the tracked beats from the grid, in milliseconds.
    pub rms_ms: f64,
    /// Share of the tracked beats within `ON_GRID` of it.
    pub on_grid: f64,
    /// Whether the track was made on this grid; see `STEADY_STRETCHES`.
    pub steady: bool,
}

impl Grid {
    pub fn period(&self) -> f64 {
        60.0 / self.bpm
    }

    /// Its lines from `from` to `to`, seconds, both included.
    pub fn lines(&self, from: f64, to: f64) -> Vec<f64> {
        let period = self.period();
        let first = ((from - self.first_beat) / period).round();
        let last = ((to - self.first_beat) / period).round();
        (first as i64..=last as i64)
            .map(|n| self.first_beat + period * n as f64)
            .filter(|&t| t >= 0.0)
            .collect()
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
    let (mut voting, mut agreeing) = (0, 0);
    for (stretch, residuals) in beats.chunks(SEED_BEATS).zip(residuals.chunks(SEED_BEATS)) {
        let own_beat = stretch_line(stretch).is_some_and(|(_, on)| on * 2 >= stretch.len());
        if stretch.len() < SEED_BEATS / 2 || !own_beat {
            continue;
        }
        voting += 1;
        if residuals.iter().filter(|r| r.abs() <= ON_GRID).count() * 2 >= residuals.len() {
            agreeing += 1;
        }
    }
    let steady = voting > 0 && agreeing as f64 >= STEADY_STRETCHES * voting as f64;

    Some(Grid {
        bpm: 60.0 / period,
        first_beat: origin.rem_euclid(period),
        rms_ms,
        on_grid,
        steady,
    })
}

/// The line through the stretch of beats that sits on one best.
fn seed(beats: &[f64]) -> Option<(f64, f64)> {
    let width = SEED_BEATS.min(beats.len());
    (0..=beats.len() - width)
        .step_by((width / 4).max(1))
        .filter_map(|start| stretch_line(&beats[start..start + width]))
        .max_by_key(|&(_, on)| on)
        .map(|(line, _)| line)
}

/// A stretch's own line, and how many of its beats are on it. Within a
/// stretch, beats are numbered from the gaps between them, which a skipped
/// beat cannot fool.
fn stretch_line(stretch: &[f64]) -> Option<((f64, f64), usize)> {
    let mut gaps: Vec<f64> = stretch.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(f64::total_cmp);
    let rough = *gaps.get(gaps.len() / 2)?;
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
    let (origin, period) = line(&numbered)?;
    let on = numbered
        .iter()
        .filter(|&&(n, b)| (b - (origin + period * n)).abs() <= ON_GRID)
        .count();
    Some(((origin, period), on))
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

    /// A long beatless breakdown, a third of the track, and the drop back on
    /// the grid: made on a grid all the same.
    #[test]
    fn a_long_breakdown_is_still_steady() {
        let period = 60.0 / 160.0;
        let mut beats: Vec<f64> = (0..140).map(|i| period * i as f64).collect();
        let mut t = period * 140.0;
        for k in 0..130 {
            t += period * (1.0 + 0.3 * (k as f64 / 7.0).sin());
            beats.push(t);
        }
        beats.extend((300..440).map(|i| period * i as f64));
        let grid = fit(&beats).unwrap();
        assert!(grid.on_grid < 0.8, "{:.0}% on grid", grid.on_grid * 100.0);
        assert!(grid.steady);
        assert!((grid.bpm - 160.0).abs() < 0.01, "{}", grid.bpm);
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
