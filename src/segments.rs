//! A track cut at its phrases, each part embedded and described: the units a
//! transition is made of. Two tracks are near when their phrases are, and
//! which phrases are near is where to mix from one into the other.
//!
//! The embedding is CLAP's audio tower, Stelly's semantic block, here over
//! eight bars instead of a track's middle 90 seconds. CLAP reads 10s windows;
//! an eight-bar phrase is 12–16s at DJ tempos, so a window and a stub of more
//! than half a window, averaged.

use crate::{key::Key, Analysis};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use stelly_audio::clap::{self, AudioEncoder};
use stelly_audio::decode::{resample, Pcm};
use stelly_audio::descriptors;

/// Shorter than this a lead-in or a tail is not worth a segment of its own.
const MIN_SECONDS: f64 = 4.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    /// Integrated loudness, LUFS: how hard this part hits.
    pub loudness: Option<f64>,
    /// Spectral centroid, Hz: how bright.
    pub brightness: Option<f64>,
    /// Detected onsets a second: how busy.
    pub onset_rate: Option<f64>,
    pub key: Option<Key>,
    /// CLAP's 512-d audio embedding, L2-normalised.
    pub embedding: Vec<f32>,
}

/// Every phrase of a track, with a lead-in before the first and a tail after
/// the last when they are long enough to mix from.
pub fn cut(pcm: &Pcm, analysis: &Analysis, encoder: &mut AudioEncoder) -> Result<Vec<Segment>> {
    let audio = resample(&pcm.samples, pcm.rate, clap::SAMPLE_RATE);
    let rate = clap::SAMPLE_RATE as f64;
    let seconds = audio.len() as f64 / rate;

    let mut bounds: Vec<f64> = Vec::new();
    if let Some(&first) = analysis.beats.first() {
        bounds.push(first);
    }
    bounds.extend(analysis.phrases.iter().copied());
    bounds.push(analysis.beats.last().copied().unwrap_or(seconds).min(seconds));
    bounds.dedup_by(|b, a| (*b - *a).abs() < 1e-6);

    let mut segments = Vec::new();
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if end - start < MIN_SECONDS {
            continue;
        }
        let slice = &audio[(start * rate) as usize..((end * rate) as usize).min(audio.len())];
        let described = descriptors::extract(slice, clap::SAMPLE_RATE)?;
        let key = match (&described.key, &described.scale, described.key_strength) {
            (Some(tonic), Some(scale), Some(strength)) => Key::new(tonic, scale, strength),
            _ => None,
        };
        segments.push(Segment {
            start,
            end,
            loudness: described.loudness_integrated,
            brightness: described.spectral_centroid,
            onset_rate: described.onset_rate,
            key,
            embedding: encoder.embed(slice)?,
        });
    }
    Ok(segments)
}
