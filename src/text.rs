//! Finding tracks by description: CLAP's text tower puts "chill downtempo"
//! in the same space as the phrases, so a track's phrases can be scored
//! against it. The same weights as Stelly's.
//!
//! Raw similarity to one phrase of text mostly measures how much a track
//! sounds like music at all, which Stelly found the hard way with its moods.
//! So a track is scored by how much nearer the description its phrases are
//! than to plain "music", and the scores are then ranked across the library.

use crate::library::Track;
use anyhow::Result;

/// CLAP's text tower, stelly-audio's.
pub use stelly_audio::clap::TextEncoder;

/// What a description is measured against.
const BASELINE: &str = "music";

/// How much each track sounds like `description`, as its rank across the
/// library: 1 the most, 0 the least. A track is its phrases' mean, so a
/// chill intro does not make a banger chill.
pub fn scores(encoder: &mut TextEncoder, tracks: &[Track], description: &str) -> Result<Vec<f64>> {
    let query = encoder.embed(description)?;
    let baseline = encoder.embed(BASELINE)?;
    let raw: Vec<f64> = tracks
        .iter()
        .map(|track| {
            let n = track.segments.len().max(1) as f64;
            track
                .segments
                .iter()
                .map(|s| dot(&s.embedding, &query) - dot(&s.embedding, &baseline))
                .sum::<f64>()
                / n
        })
        .collect();
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &b| raw[a].total_cmp(&raw[b]));
    let mut ranks = vec![0.0; raw.len()];
    for (rank, &i) in order.iter().enumerate() {
        ranks[i] = rank as f64 / (raw.len().max(2) - 1) as f64;
    }
    Ok(ranks)
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x * y) as f64).sum()
}
