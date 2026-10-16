//! stelly-mix: what a DJ needs to know about a track before mixing it. Where
//! the beats are, how steady they are, where bars and phrases start, and what
//! key it is in.
//!
//! Decoding and the descriptors come from `stelly-audio`, shared with Stelly;
//! everything about time, beats onwards, lives here.

pub mod bands;
pub mod beats;
pub mod decode;
pub mod eval;
pub mod grid;
pub mod key;
pub mod library;
pub mod models;
pub mod render;
pub mod segments;
pub mod structure;
pub mod tempo;
pub mod transitions;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use stelly_audio::decode::Pcm;
use stelly_audio::descriptors;

pub use grid::Grid;
pub use key::Key;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    pub seconds: f64,
    /// The steady grid through the beats, absent with too few of them.
    pub grid: Option<Grid>,
    pub key: Option<Key>,
    /// Integrated loudness, LUFS, for gain matching.
    pub loudness: Option<f64>,
    /// Beats, seconds: the grid's lines on a steady track, the tracked beats,
    /// which follow a drifting tempo, otherwise.
    pub beats: Vec<f64>,
    /// Every fourth beat from the first downbeat.
    pub downbeats: Vec<f64>,
    pub downbeat_confidence: f64,
    /// Where each eight-bar phrase starts, seconds.
    pub phrases: Vec<f64>,
    /// Where the music changes enough to call it a new section, seconds.
    pub sections: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// The octave tempos are reported in, `[min, max)`; see `tempo::DJ_BPM`.
    pub bpm_range: (f64, f64),
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bpm_range: tempo::DJ_BPM,
        }
    }
}

/// Everything, from a decoded track.
pub fn analyse(pcm: &Pcm, options: &Options) -> Result<Analysis> {
    let described = descriptors::extract(&pcm.samples, pcm.rate)?;
    let onsets = descriptors::onsets(&pcm.samples, pcm.rate);
    let tempo::Choice { beats, grid, .. } =
        tempo::choose(&onsets, described.bpm, options.bpm_range).context("found no tempo")?;
    // A track made on a grid has its beats on it, breakdowns included, where
    // the tracker wanders; and counting bars on them cannot be thrown by a
    // beat the tracker dropped.
    let beats = match (&grid, beats.first(), beats.last()) {
        (Some(grid), Some(&first), Some(&last)) if grid.steady => grid.lines(first, last),
        _ => beats,
    };

    let features = bands::per_beat(&pcm.samples, pcm.rate, &beats);
    let structure = structure::analyse(&features);

    let downbeats: Vec<f64> = beats
        .iter()
        .skip(structure.downbeat)
        .step_by(structure::BEATS_PER_BAR)
        .copied()
        .collect();
    let phrases = downbeats
        .iter()
        .skip(structure.phrase_offset)
        .step_by(structure::PHRASE_BARS)
        .copied()
        .collect();
    let sections = structure
        .sections
        .iter()
        .filter_map(|&bar| downbeats.get(bar).copied())
        .collect();

    let key = match (&described.key, &described.scale, described.key_strength) {
        (Some(tonic), Some(scale), Some(strength)) => Key::new(tonic, scale, strength),
        _ => None,
    };

    Ok(Analysis {
        seconds: pcm.seconds(),
        grid,
        key,
        loudness: described.loudness_integrated,
        beats,
        downbeats,
        downbeat_confidence: structure.downbeat_confidence,
        phrases,
        sections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beats::tests::{kick, RATE};

    /// Kicks on every beat from a two-beat pickup, a chord changing on every
    /// bar after it, and noise switching on and off every eight bars from bar 3.
    fn song(bpm: f64) -> (Vec<f32>, f64) {
        let period = 60.0 / bpm;
        let (pickup, bars) = (2, 40);
        let seconds = (pickup + 4 * bars) as f64 * period + 1.0;
        let mut out = vec![0f32; (seconds * RATE as f64) as usize];
        let start = 0.25;
        for beat in 0..pickup + 4 * bars {
            kick(&mut out, ((start + beat as f64 * period) * RATE as f64) as usize, 0.6);
        }
        let first_bar = start + pickup as f64 * period;
        let roots = [220.0, 196.0, 174.6, 246.9];
        let mut noise = 1u32;
        for (i, slot) in out.iter_mut().enumerate() {
            let t = i as f64 / RATE as f64;
            if t < first_bar {
                continue;
            }
            let bar = ((t - first_bar) / (4.0 * period)) as usize;
            if bar >= bars {
                break;
            }
            let root = roots[bar % roots.len()];
            for ratio in [1.0, 1.25, 1.5] {
                *slot += 0.08 * (2.0 * std::f64::consts::PI * root * ratio * t).sin() as f32;
            }
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if bar >= 3 && ((bar - 3) / 8).is_multiple_of(2) {
                *slot += 0.08 * (noise as f32 / u32::MAX as f32 - 0.5);
            }
        }
        (out, first_bar)
    }

    #[test]
    fn downbeats_and_phrases_of_a_song() {
        let (samples, first_bar) = song(126.0);
        let analysis = analyse(&Pcm { samples, rate: RATE }, &Options::default()).unwrap();
        let bar = 4.0 * 60.0 / 126.0;

        let first = analysis.downbeats[0];
        let off = ((first - first_bar) / bar).fract().abs();
        assert!(off.min(1.0 - off) * bar < 0.01, "first downbeat at {first:.3}s, bar line at {first_bar:.3}s");

        let phrase = first_bar + 3.0 * bar;
        for p in &analysis.phrases {
            let off = ((p - phrase) / (8.0 * bar)).rem_euclid(1.0);
            assert!(off.min(1.0 - off) * 8.0 * bar < 0.01, "phrase at {p:.3}s, expected {phrase:.3}s + 8 bars");
        }
        assert!(analysis.phrases.len() >= 4, "{:?}", analysis.phrases);
    }
}
