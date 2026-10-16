//! Choosing the tempo by tracking with it.
//!
//! The onset envelope's autocorrelation peaks at the pulse and at everything
//! related to it: double, half, and on music with a triplet feel two thirds,
//! which can come out on top. Stelly only needs the right family, so takes the
//! tallest peak; a beat grid needs the right one. So each strong candidate is
//! tracked, and the one whose beats sit best on a steady grid wins.

use crate::beats;
use crate::grid::{self, Grid};
use stelly_audio::descriptors::Onsets;

const MIN_BPM: f64 = 40.0;
const MAX_BPM: f64 = 220.0;
/// Peaks of the autocorrelation to try.
const CANDIDATES: usize = 6;
/// The octave a tempo is reported in. Below it a tempo is read as half-time
/// and doubled, so a 125 BPM track tracked on its half-time pulse still says
/// 125; above it, as eighth notes, and halved, so a rock track whose hi-hats
/// make the steadiest grid says 112, not 224. Drum & bass stays at 174.
pub const DJ_BPM: (f64, f64) = (75.0, 175.0);
/// Shares of beats on the grid closer than this count as a tie, broken
/// towards 120 BPM as Stelly's estimate is.
const TIE: f64 = 0.05;

pub struct Choice {
    pub bpm: f64,
    pub beats: Vec<f64>,
    pub grid: Option<Grid>,
}

/// Track with each candidate plus the `hint`, keep the best.
pub fn choose(onsets: &Onsets, hint: Option<f64>) -> Option<Choice> {
    let mut tried: Vec<Choice> = candidates(onsets)
        .into_iter()
        .chain(hint)
        .map(|bpm| {
            let beats = beats::track(onsets, bpm);
            let grid = grid::fit(&beats);
            Choice { bpm, beats, grid }
        })
        .collect();
    let share = |c: &Choice| c.grid.as_ref().map_or(0.0, |g| g.on_grid);
    let best = tried.iter().map(share).fold(0.0, f64::max);
    let prior = |c: &Choice| -(c.bpm / 120.0).log2().abs();
    tried.retain(|c| share(c) >= best - TIE);
    let mut choice = tried.into_iter().max_by(|a, b| prior(a).total_cmp(&prior(b)))?;

    choice.bpm = choice.grid.as_ref().map_or(choice.bpm, |g| g.bpm);
    while choice.bpm < DJ_BPM.0 && choice.beats.len() >= 2 {
        choice.beats = double(&choice.beats);
        choice.grid = grid::fit(&choice.beats);
        choice.bpm *= 2.0;
    }
    while choice.bpm >= DJ_BPM.1 && choice.beats.len() >= 16 {
        choice.beats = halve(onsets, &choice.beats);
        choice.grid = grid::fit(&choice.beats);
        choice.bpm /= 2.0;
    }
    choice.bpm = choice.grid.as_ref().map_or(choice.bpm, |g| g.bpm);
    Some(choice)
}

/// The tallest peaks of the envelope's autocorrelation, as BPM, each refined
/// below a frame by a parabola through it and its neighbours.
pub fn candidates(onsets: &Onsets) -> Vec<f64> {
    let envelope = &onsets.envelope;
    let n = envelope.len();
    let mean = envelope.iter().sum::<f64>() / n.max(1) as f64;
    let centred: Vec<f64> = envelope.iter().map(|v| v - mean).collect();
    let energy: f64 = centred.iter().map(|v| v * v).sum();
    let shortest = (60.0 * onsets.fps / MAX_BPM).floor() as usize;
    let longest = ((60.0 * onsets.fps / MIN_BPM).ceil() as usize).min(n / 2);
    if energy < 1e-12 || shortest < 2 || longest <= shortest + 2 {
        return Vec::new();
    }

    let autocorrelation: Vec<f64> = (0..=longest + 1)
        .map(|lag| centred[..n - lag].iter().zip(&centred[lag..]).map(|(a, b)| a * b).sum::<f64>() / energy)
        .collect();
    let mut peaks: Vec<(f64, f64)> = (shortest..=longest)
        .filter(|&lag| {
            let v = autocorrelation[lag];
            v > 0.0 && v > autocorrelation[lag - 1] && v >= autocorrelation[lag + 1]
        })
        .map(|lag| {
            let (a, b, c) = (autocorrelation[lag - 1], autocorrelation[lag], autocorrelation[lag + 1]);
            let denominator = a - 2.0 * b + c;
            let shift = if denominator.abs() > 1e-12 {
                (0.5 * (a - c) / denominator).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            (60.0 * onsets.fps / (lag as f64 + shift), b)
        })
        .collect();
    peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
    peaks.into_iter().take(CANDIDATES).map(|(bpm, _)| bpm).collect()
}

/// A beat between every two, for a tempo tracked at half-time.
fn double(beats: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(beats.len() * 2);
    for pair in beats.windows(2) {
        out.push(pair[0]);
        out.push((pair[0] + pair[1]) / 2.0);
    }
    out.extend(beats.last());
    out
}

/// Every other beat, the half with the stronger onsets: halving a grid
/// rather than tracking at the slower tempo, which latches onto backbeats.
fn halve(onsets: &Onsets, beats: &[f64]) -> Vec<f64> {
    let weight = |phase: usize| -> f64 {
        beats.iter().skip(phase).step_by(2).map(|&b| beats::strength(onsets, b)).sum()
    };
    let phase = usize::from(weight(1) > weight(0));
    beats.iter().skip(phase).step_by(2).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beats::tests::{clicks, kick, RATE};
    use stelly_audio::descriptors;

    #[test]
    fn a_triplet_feel_does_not_win() {
        // Kicks at 125 BPM, and louder hits every third eighth-note triplet,
        // so a pulse at two thirds of the tempo correlates strongly too.
        let (bpm, seconds) = (125.0, 60.0);
        let mut audio = clicks(bpm, 0.2, seconds);
        let mut t = 0.2;
        while t < seconds {
            kick(&mut audio, (t * RATE as f64) as usize, 1.0);
            t += 60.0 / (bpm * 2.0 / 3.0);
        }
        let onsets = descriptors::onsets(&audio, RATE);
        let choice = choose(&onsets, Some(bpm * 2.0 / 3.0)).unwrap();
        assert!((choice.bpm - bpm).abs() < 0.05, "chose {}", choice.bpm);
    }

    #[test]
    fn halving_keeps_the_kicks() {
        // Kicks at 112 BPM, quieter hats on the eighth notes between them,
        // and a beat on every eighth note to halve.
        let (bpm, offset, seconds) = (112.0, 0.4, 30.0);
        let mut audio = clicks(bpm, offset, seconds);
        let eighth = 30.0 / bpm;
        let mut t = offset + eighth;
        while t < seconds {
            kick(&mut audio, (t * RATE as f64) as usize, 0.3);
            t += 2.0 * eighth;
        }
        let onsets = descriptors::onsets(&audio, RATE);
        for start in [0, 1] {
            let eighths: Vec<f64> = (start..100).map(|i| offset + i as f64 * eighth).collect();
            for b in halve(&onsets, &eighths) {
                let off = (b - offset) / (2.0 * eighth);
                assert!((off - off.round()).abs() < 0.01, "kept a hat at {b:.3}s");
            }
        }
    }

    #[test]
    fn half_time_is_doubled() {
        let onsets = descriptors::onsets(&clicks(62.5, 0.3, 60.0), RATE);
        let choice = choose(&onsets, None).unwrap();
        assert!((choice.bpm - 125.0).abs() < 0.05, "chose {}", choice.bpm);
    }
}
