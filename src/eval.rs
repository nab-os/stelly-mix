//! Scoring against annotated beats, with the measures beat tracking papers
//! report, so numbers here can be set beside theirs.
//!
//! - F-measure: an estimated beat within 70ms of an annotated one is a hit,
//!   each annotation matched at most once.
//! - Tempo: Acc1 is within 4% of the annotated tempo; Acc2 also forgives
//!   the octave and triple errors, ×2, ×½, ×3, ×⅓.
//!
//! Annotations are the `.beats` files of the CPJKU Ballroom set and others in
//! its format: one beat a line, its time, then its position in the bar,
//! 1 for a downbeat.

use anyhow::{Context, Result};
use std::path::Path;

pub const TOLERANCE: f64 = 0.070;
const TEMPO_TOLERANCE: f64 = 0.04;

#[derive(Debug, Clone, Default)]
pub struct Truth {
    pub beats: Vec<f64>,
    pub downbeats: Vec<f64>,
}

impl Truth {
    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut truth = Truth::default();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let mut fields = line.split_whitespace();
            let time: f64 = fields
                .next()
                .and_then(|t| t.parse().ok())
                .with_context(|| format!("{}: bad line {line:?}", path.display()))?;
            truth.beats.push(time);
            if fields.next().and_then(|p| p.parse::<f64>().ok()) == Some(1.0) {
                truth.downbeats.push(time);
            }
        }
        Ok(truth)
    }

    /// From the median gap between annotated beats.
    pub fn bpm(&self) -> Option<f64> {
        median(self.beats.windows(2).map(|w| w[1] - w[0]).collect()).map(|gap| 60.0 / gap)
    }
}

/// Harmonic mean of precision and recall, 0 when either list is empty.
pub fn f_measure(estimated: &[f64], annotated: &[f64]) -> f64 {
    if estimated.is_empty() || annotated.is_empty() {
        return 0.0;
    }
    let hits = matches(estimated, annotated) as f64;
    let (precision, recall) = (hits / estimated.len() as f64, hits / annotated.len() as f64);
    if hits == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

/// One-to-one matches within the tolerance. Both lists are sorted and beats
/// are far wider apart than twice the tolerance, so a merge finds them.
fn matches(estimated: &[f64], annotated: &[f64]) -> usize {
    let (mut i, mut j, mut hits) = (0, 0, 0);
    while i < estimated.len() && j < annotated.len() {
        let gap = estimated[i] - annotated[j];
        if gap.abs() <= TOLERANCE {
            hits += 1;
            i += 1;
            j += 1;
        } else if gap < 0.0 {
            i += 1;
        } else {
            j += 1;
        }
    }
    hits
}

/// (Acc1, Acc2) for one estimate.
pub fn tempo_accuracy(estimated: f64, annotated: f64) -> (bool, bool) {
    let close = |factor: f64| (estimated - factor * annotated).abs() <= TEMPO_TOLERANCE * factor * annotated;
    (close(1.0), [1.0, 2.0, 0.5, 3.0, 1.0 / 3.0].into_iter().any(close))
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f_measure_counts_each_annotation_once() {
        let truth = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(f_measure(&truth, &truth), 1.0);
        // Two estimates near one annotation make one hit.
        let doubled = [1.0, 1.05, 2.0, 3.0, 4.0];
        assert!((f_measure(&doubled, &truth) - 2.0 * 0.8 / 1.8).abs() < 1e-9);
        // Off by more than the tolerance is a miss.
        assert_eq!(f_measure(&[1.1, 2.1, 3.1, 4.1], &truth), 0.0);
        // Half the tempo: perfect precision, half the recall.
        assert!((f_measure(&[1.0, 3.0], &truth) - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn tempo_octaves_count_for_acc2_only() {
        assert_eq!(tempo_accuracy(121.0, 120.0), (true, true));
        assert_eq!(tempo_accuracy(240.0, 120.0), (false, true));
        assert_eq!(tempo_accuracy(80.0, 120.0), (false, false));
        assert_eq!(tempo_accuracy(40.0, 120.0), (false, true));
    }

    #[test]
    fn reads_ballroom_annotations() {
        let path = std::env::temp_dir().join(format!("stelly-mix-truth-{}.beats", std::process::id()));
        std::fs::write(&path, "0.19 4\n0.85 1\n1.47 2\n2.08 3\n2.68 4\n3.30 1\n").unwrap();
        let truth = Truth::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(truth.beats.len(), 6);
        assert_eq!(truth.downbeats, vec![0.85, 3.30]);
        // Gaps 0.66, 0.62, 0.61, 0.60, 0.62: the median is 0.62.
        assert!((truth.bpm().unwrap() - 60.0 / 0.62).abs() < 1e-9);
    }
}
