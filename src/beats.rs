//! Where the beats fall, given the tempo: Ellis's dynamic programme over the
//! onset envelope (Ellis 2007, as librosa does it). Every frame scores how
//! strong an onset it is plus the best earlier beat about one period back,
//! penalised for how far off the period that gap is; the best path through
//! the last frames is the beat sequence.
//!
//! It follows a tempo that drifts, which a fixed grid cannot, and it is
//! frame-accurate only, 10ms at 48kHz. `grid` fits a steady grid through the
//! result for the sub-frame precision syncing wants.

use stelly_audio::descriptors::Onsets;

/// How hard a gap off the period is penalised, librosa's default. Higher keeps
/// the tracker on tempo through breakdowns, lower lets it follow rubato.
const TIGHTNESS: f64 = 100.0;

/// Frames from the start of an STFT window to where an attack registers in
/// the onset envelope. Measured on click tracks, see the tests.
const ONSET_LAG_FRAMES: f64 = 3.0;

/// Beat times in seconds, from an onset envelope and a tempo estimate.
pub fn track(onsets: &Onsets, bpm: f64) -> Vec<f64> {
    let envelope = &onsets.envelope;
    let n = envelope.len();
    let period = 60.0 * onsets.fps / bpm;
    if n < (4.0 * period) as usize || !period.is_finite() {
        return Vec::new();
    }

    let local = local_score(envelope, period);
    let (shortest, longest) = ((period / 2.0).round() as usize, (2.0 * period).round() as usize);

    let mut cumulative = vec![0f64; n];
    let mut back: Vec<Option<usize>> = vec![None; n];
    for t in 0..n {
        let mut best: Option<(usize, f64)> = None;
        for gap in shortest..=longest.min(t) {
            let before = t - gap;
            let score = cumulative[before] - TIGHTNESS * (gap as f64 / period).ln().powi(2);
            if best.is_none_or(|(_, s)| score > s) {
                best = Some((before, score));
            }
        }
        cumulative[t] = local[t] + best.map_or(0.0, |(_, s)| s.max(0.0));
        back[t] = best.filter(|&(_, s)| s > 0.0).map(|(b, _)| b);
    }

    let Some(last) = last_beat(&cumulative) else {
        return Vec::new();
    };
    let mut frames = vec![last];
    while let Some(before) = back[*frames.last().unwrap()] {
        frames.push(before);
    }
    frames.reverse();

    let frames = trim(&frames, &local);
    frames
        .iter()
        .map(|&f| (f as f64 + ONSET_LAG_FRAMES) / onsets.fps)
        .collect()
}

/// The envelope over its standard deviation, smoothed with a Gaussian a
/// thirty-second of a period wide, so a beat need not land on the exact frame.
fn local_score(envelope: &[f64], period: f64) -> Vec<f64> {
    let n = envelope.len() as f64;
    let mean = envelope.iter().sum::<f64>() / n;
    let std = (envelope.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt().max(1e-12);

    let sigma = (period / 32.0).max(0.5);
    let half = (3.0 * sigma).ceil() as isize;
    let kernel: Vec<f64> = (-half..=half)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect();

    (0..envelope.len() as isize)
        .map(|t| {
            kernel
                .iter()
                .enumerate()
                .filter_map(|(k, w)| {
                    let i = t + k as isize - half;
                    envelope.get(usize::try_from(i).ok()?).map(|v| w * v / std)
                })
                .sum()
        })
        .collect()
}

/// The last frame whose cumulative score is a local peak at least half the
/// median peak, so the path does not end on a fade-out's last whisper.
fn last_beat(cumulative: &[f64]) -> Option<usize> {
    let peaks: Vec<usize> = (1..cumulative.len().saturating_sub(1))
        .filter(|&t| cumulative[t] > cumulative[t - 1] && cumulative[t] >= cumulative[t + 1])
        .collect();
    let mut heights: Vec<f64> = peaks.iter().map(|&t| cumulative[t]).collect();
    heights.sort_by(f64::total_cmp);
    let median = *heights.get(heights.len() / 2)?;
    peaks.into_iter().rev().find(|&t| cumulative[t] >= 0.5 * median)
}

/// Drop beats from either end while they sit on near-silence: the tracker
/// keeps counting through an intro or an outro with nothing in it.
fn trim(frames: &[usize], local: &[f64]) -> Vec<usize> {
    let strength: Vec<f64> = frames.iter().map(|&f| local[f]).collect();
    let rms = (strength.iter().map(|v| v * v).sum::<f64>() / strength.len().max(1) as f64).sqrt();
    let threshold = 0.5 * rms;
    let start = strength.iter().position(|&s| s >= threshold).unwrap_or(0);
    let end = strength.iter().rposition(|&s| s >= threshold).map_or(0, |e| e + 1);
    frames[start..end.max(start)].to_vec()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use stelly_audio::descriptors;

    pub const RATE: u32 = 44_100;

    /// A kick: a 55Hz sine dropping fast in pitch and level.
    pub fn kick(out: &mut [f32], at: usize, gain: f32) {
        for i in 0..(0.15 * RATE as f64) as usize {
            let Some(slot) = out.get_mut(at + i) else { break };
            let t = i as f32 / RATE as f32;
            let pitch = 55.0 + 120.0 * (-t * 40.0).exp();
            *slot += gain * (-t * 25.0).exp() * (2.0 * std::f32::consts::PI * pitch * t).sin();
        }
    }

    /// A click track at `bpm` whose first beat is at `offset` seconds.
    pub fn clicks(bpm: f64, offset: f64, seconds: f64) -> Vec<f32> {
        let mut out = vec![0f32; (seconds * RATE as f64) as usize];
        let mut beat = offset;
        while beat < seconds {
            kick(&mut out, (beat * RATE as f64) as usize, 0.8);
            beat += 60.0 / bpm;
        }
        out
    }

    #[test]
    fn beats_land_on_the_clicks() {
        for (bpm, offset) in [(128.0, 0.31), (174.0, 0.05), (93.0, 1.2)] {
            let audio = clicks(bpm, offset, 60.0);
            let onsets = descriptors::onsets(&audio, RATE);
            let beats = track(&onsets, bpm);
            let period = 60.0 / bpm;
            assert!(beats.len() as f64 > 55.0 / period, "{bpm}: {} beats", beats.len());
            for b in &beats {
                let phase = ((b - offset) / period).round();
                let error = b - (offset + phase * period);
                assert!(error.abs() < 0.008, "{bpm}: beat at {b:.3}s is {:.1}ms off", error * 1e3);
            }

            // The grid averages the frame quantisation away.
            let grid = crate::grid::fit(&beats).unwrap();
            assert!((grid.bpm - bpm).abs() < 0.02, "{bpm} fitted as {}", grid.bpm);
            let phase = (grid.first_beat - offset.rem_euclid(period)).abs();
            assert!(phase.min(period - phase) < 0.002, "{bpm}: grid {:.1}ms off", phase * 1e3);
            assert!(grid.steady);
        }
    }
}
