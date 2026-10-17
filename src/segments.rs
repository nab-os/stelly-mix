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
    /// CLAP's 512-d audio embedding, L2-normalised. Stored as 16-bit floats,
    /// see `compact`.
    #[serde(with = "compact")]
    pub embedding: Vec<f32>,
}

/// An embedding on disk as base64 of little-endian 16-bit floats: 1.4KB for
/// 512 dimensions where JSON numbers took 6–7KB, and a cosine unchanged to
/// the fourth decimal. Entries written before read as the numbers they hold.
pub mod compact {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(embedding: &[f32], serializer: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = embedding.iter().flat_map(|v| half::f16::from_f32(*v).to_le_bytes()).collect();
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<f32>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Compact(String),
            Numbers(Vec<f32>),
        }
        match Stored::deserialize(deserializer)? {
            Stored::Numbers(numbers) => Ok(numbers),
            Stored::Compact(text) => {
                let bytes = STANDARD.decode(text).map_err(serde::de::Error::custom)?;
                if bytes.len() % 2 != 0 {
                    return Err(serde::de::Error::custom("an embedding of half a float"));
                }
                Ok(bytes.chunks_exact(2).map(|b| half::f16::from_le_bytes([b[0], b[1]]).to_f32()).collect())
            }
        }
    }
}

/// Every phrase of a track, with a lead-in before the first and a tail after
/// the last when they are long enough to mix from.
pub fn cut(pcm: &Pcm, analysis: &Analysis, encoder: &mut AudioEncoder) -> Result<Vec<Segment>> {
    let audio = resample(&pcm.samples, pcm.rate, clap::SAMPLE_RATE);
    let rate = clap::SAMPLE_RATE as f64;
    let seconds = audio.len() as f64 / rate;

    // The lead-in from the first downbeat, so that it starts on a bar like
    // every phrase; from the first beat only if there are no downbeats.
    let mut bounds: Vec<f64> = Vec::new();
    if let Some(&first) = analysis.downbeats.first().or(analysis.beats.first()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeddings_store_small_and_read_back_close_in_either_form() {
        let embedding: Vec<f32> = (0..512).map(|i| ((i as f32) * 0.37).sin() / 16.0).collect();
        let segment = Segment {
            start: 0.0,
            end: 1.0,
            loudness: None,
            brightness: None,
            onset_rate: None,
            key: None,
            embedding: embedding.clone(),
        };
        let compact = serde_json::to_string(&segment).unwrap();
        let mut numbers = serde_json::to_value(&segment).unwrap();
        numbers["embedding"] = serde_json::json!(embedding);
        let numbers = numbers.to_string();
        assert!(compact.len() * 4 < numbers.len(), "{} against {}", compact.len(), numbers.len());

        let back: Segment = serde_json::from_str(&compact).unwrap();
        let worst = back.embedding.iter().zip(&embedding).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(worst < 1e-4, "{worst}");
        let old: Segment = serde_json::from_str(&numbers).unwrap();
        assert_eq!(old.embedding, embedding);
    }
}
