//! Where to go next: from a phrase late in the playing track into a phrase
//! early in another, scored on what a DJ weighs.
//!
//! - **Sound**: CLAP cosine between the two phrases, so a transition joins
//!   passages that sound alike rather than tracks that do on average. Within
//!   a genre every pair is 0.93–0.96, so each track's best pair is scored
//!   against the other tracks' best: two deviations above their mean is a 1.
//! - **Tempo**: how far the incoming track must be sped up or slowed down,
//!   octaves folded away; past `max_stretch` it is not a candidate at all.
//! - **Key**: steps on the Camelot wheel between the two tracks.
//! - **Energy**: loudness of the incoming phrase against the outgoing one.
//!
//! The weights are a first guess, to be tuned by ear.

use crate::library::Track;
use serde::Serialize;

const SOUND: f64 = 0.55;
const TEMPO: f64 = 0.15;
const KEY: f64 = 0.2;
const ENERGY: f64 = 0.1;
/// Below this a key estimate is a guess, and scores as neither good nor bad.
const KEY_STRENGTH: f64 = 0.5;
/// A loudness step this large in either direction scores nothing, LU.
const ENERGY_RANGE: f64 = 6.0;

#[derive(Debug, Clone)]
pub struct Options {
    /// Largest tempo change allowed, as a fraction: 0.06 is ±6%.
    pub max_stretch: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self { max_stretch: 0.06 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Transition {
    /// Index of the incoming track in the list it was chosen from.
    pub to: usize,
    /// Segment of the playing track to mix out of.
    pub out: usize,
    /// Segment of the incoming track to mix into.
    pub into: usize,
    pub score: f64,
    pub sound: f64,
    /// Incoming tempo over outgoing, octaves folded: 1.02 is 2% faster.
    pub ratio: f64,
    pub key_steps: Option<u32>,
    /// Incoming loudness minus outgoing, LU.
    pub energy: Option<f64>,
}

/// The best transition into each of `others`, best first. The playing track
/// is mixed out of from its second half, the next one mixed into in its first.
pub fn from(playing: &Track, others: &[Track], options: &Options) -> Vec<Transition> {
    let Some(bpm) = playing.bpm() else { return Vec::new() };
    let outs = half(playing, false);

    // Each track's best pair of phrases first, by how alike they sound and
    // how evenly the energy carries over.
    let mut best: Vec<(Transition, f64, f64)> = others
        .iter()
        .enumerate()
        .filter(|(_, next)| next.path != playing.path)
        .filter_map(|(to, next)| {
            let ratio = fold(next.bpm()? / bpm);
            let stretch = (ratio - 1.0).abs();
            if stretch > options.max_stretch {
                return None;
            }
            let key_steps = match (&playing.analysis.key, &next.analysis.key) {
                (Some(a), Some(b)) if a.strength >= KEY_STRENGTH && b.strength >= KEY_STRENGTH => a.distance(b),
                _ => None,
            };
            let key_score = match key_steps {
                Some(0) => 1.0,
                Some(1) => 0.8,
                Some(2) => 0.4,
                Some(_) => 0.0,
                None => 0.5,
            };
            let fixed = TEMPO * (1.0 - stretch / options.max_stretch) + KEY * key_score;

            let ins = half(next, true);
            outs.iter()
                .flat_map(|&out| ins.iter().map(move |&into| (out, into)))
                .map(|(out, into)| {
                    let (a, b) = (&playing.segments[out], &next.segments[into]);
                    let sound = cosine(&a.embedding, &b.embedding);
                    let energy = a.loudness.zip(b.loudness).map(|(a, b)| b - a);
                    let energy_score = energy.map_or(0.5, |e| 1.0 - (e.abs() / ENERGY_RANGE).min(1.0));
                    let transition = Transition {
                        to,
                        out,
                        into,
                        score: 0.0,
                        sound,
                        ratio,
                        key_steps,
                        energy,
                    };
                    (transition, fixed, energy_score)
                })
                .max_by(|a, b| (a.0.sound + 0.01 * a.2).total_cmp(&(b.0.sound + 0.01 * b.2)))
        })
        .collect();

    // Then sound scored against the other tracks' best: within a genre every
    // pair is 0.93–0.96, so the raw cosine would hardly count.
    let n = best.len().max(1) as f64;
    let mean = best.iter().map(|b| b.0.sound).sum::<f64>() / n;
    let spread = (best.iter().map(|b| (b.0.sound - mean).powi(2)).sum::<f64>() / n).sqrt().max(1e-6);
    for (transition, fixed, energy_score) in &mut best {
        let sound_score = (((transition.sound - mean) / spread + 2.0) / 4.0).clamp(0.0, 1.0);
        transition.score = SOUND * sound_score + *fixed + ENERGY * *energy_score;
    }
    let mut best: Vec<Transition> = best.into_iter().map(|b| b.0).collect();
    best.sort_by(|a, b| b.score.total_cmp(&a.score));
    best
}

/// Segment indices in a track's first half, or its second. A track too short
/// to have both gets all of them.
fn half(track: &Track, first: bool) -> Vec<usize> {
    let middle = track.analysis.seconds / 2.0;
    let picked: Vec<usize> = track
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| if first { s.end <= middle + 1e-6 } else { s.start >= middle - 1e-6 })
        .map(|(i, _)| i)
        .collect();
    if picked.is_empty() {
        (0..track.segments.len()).collect()
    } else {
        picked
    }
}

/// A tempo ratio moved by octaves to within a tritone of 1: a 174 track
/// after an 87 one is a 1.0, not a 2.0.
fn fold(mut ratio: f64) -> f64 {
    while ratio > std::f64::consts::SQRT_2 {
        ratio /= 2.0;
    }
    while ratio < std::f64::consts::FRAC_1_SQRT_2 {
        ratio *= 2.0;
    }
    ratio
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (dot / (norm(a) * norm(b)).max(1e-9)) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octaves_fold_away() {
        assert!((fold(174.0 / 87.0) - 1.0).abs() < 1e-9);
        assert!((fold(160.0 / 155.0) - 160.0 / 155.0).abs() < 1e-9);
        assert!((fold(80.0 / 155.0) - 160.0 / 155.0).abs() < 1e-9);
    }
}
