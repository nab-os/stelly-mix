//! Bars and phrases from beat-synchronous features.
//!
//! Music changes on its downbeats: a chord, a new layer, a fill landing. So
//! for each beat, compare the bar before it with the bar after it; the one of
//! the four beat positions where that contrast is highest on average is the
//! downbeat. A steady loop gives no contrast at any position, so the evidence
//! comes from wherever the track changes, which is also what a DJ counts from.
//!
//! Phrases are the same thing a level up: bars before against bars after,
//! peaks of that are where sections start, and the bar position modulo eight
//! with the most contrast is where the eight-bar phrase grid sits.

use crate::bands::DIMS;
use serde::Serialize;

pub const BEATS_PER_BAR: usize = 4;
pub const PHRASE_BARS: usize = 8;
/// Bars either side of a bar line when looking for a section change.
const SECTION_WIDTH: usize = 4;

#[derive(Debug, Clone, Serialize)]
pub struct Structure {
    /// Index into the beats of the first downbeat.
    pub downbeat: usize,
    /// How much the chosen beat position beat the runner-up, 0..1. Low means
    /// the track never changed enough to tell.
    pub downbeat_confidence: f64,
    /// Bar, counted from the first downbeat, where the first full eight-bar
    /// phrase starts.
    pub phrase_offset: usize,
    /// Bars where a section starts, counted from the first downbeat.
    pub sections: Vec<usize>,
}

pub fn analyse(beats: &[[f64; DIMS]]) -> Structure {
    let (downbeat, downbeat_confidence) = downbeat(beats);

    let bars: Vec<[f64; DIMS]> = beats[downbeat.min(beats.len())..]
        .chunks_exact(BEATS_PER_BAR)
        .map(mean)
        .collect();
    let contrast: Vec<Option<f64>> = (0..bars.len()).map(|k| novelty(&bars, k, SECTION_WIDTH)).collect();

    let phrase_offset = (0..PHRASE_BARS)
        .max_by(|&a, &b| {
            let score = |o: usize| average(contrast.iter().skip(o).step_by(PHRASE_BARS).flatten().copied());
            score(a).total_cmp(&score(b))
        })
        .unwrap_or(0);

    Structure {
        downbeat,
        downbeat_confidence,
        phrase_offset,
        sections: peaks(&contrast),
    }
}

/// The beat position, 0..4, whose bar lines show the most change.
fn downbeat(beats: &[[f64; DIMS]]) -> (usize, f64) {
    let contrast: Vec<Option<f64>> = (0..beats.len()).map(|b| novelty(beats, b, BEATS_PER_BAR)).collect();
    let mut scores: Vec<(usize, f64)> = (0..BEATS_PER_BAR)
        .map(|p| (p, average(contrast.iter().skip(p).step_by(BEATS_PER_BAR).flatten().copied())))
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, runner_up) = (scores[0], scores[1]);
    let confidence = if best.1 > 0.0 { (best.1 - runner_up.1) / best.1 } else { 0.0 };
    (best.0, confidence)
}

/// Distance between the mean of `width` rows before `at` and `width` after.
fn novelty(rows: &[[f64; DIMS]], at: usize, width: usize) -> Option<f64> {
    if at < width || at + width > rows.len() {
        return None;
    }
    let (before, after) = (mean(&rows[at - width..at]), mean(&rows[at..at + width]));
    Some(before.iter().zip(&after).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt())
}

/// Local maxima within a phrase either side that stand half a deviation
/// above the mean.
fn peaks(contrast: &[Option<f64>]) -> Vec<usize> {
    let values: Vec<f64> = contrast.iter().flatten().copied().collect();
    if values.is_empty() {
        return Vec::new();
    }
    let n = values.len() as f64;
    let m = values.iter().sum::<f64>() / n;
    let threshold = m + 0.5 * (values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / n).sqrt();
    let reach = PHRASE_BARS / 2;

    // A plateau counts once, at its first bar.
    (0..contrast.len())
        .filter(|&k| {
            let Some(v) = contrast[k] else { return false };
            v >= threshold
                && (k.saturating_sub(reach)..(k + reach + 1).min(contrast.len()))
                    .filter(|&j| j != k)
                    .all(|j| contrast[j].is_none_or(|u| if j < k { u < v } else { u <= v }))
        })
        .collect()
}

fn mean(rows: &[[f64; DIMS]]) -> [f64; DIMS] {
    let mut out = [0f64; DIMS];
    for row in rows {
        for (o, v) in out.iter_mut().zip(row) {
            *o += v / rows.len() as f64;
        }
    }
    out
}

fn average(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0usize), |(s, c), v| (s + v, c + 1));
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}
