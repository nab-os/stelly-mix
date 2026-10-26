//! Drops: where the kick and the bass come back after a breakdown or a build.
//!
//! The low end is what a drop is made of. A breakdown takes the kick and the
//! sub out and leaves the pads and the riser; the drop is the bar they land
//! on again. So for each bar line, the low end of the bars before it against
//! the bars after: a drop is a bar where it was well under its usual level
//! and comes back to that level at once.
//!
//! Nothing here knows about genre. Outside music made of breakdowns, what
//! it finds is the same shape: the full band back in after a quiet part.

use crate::bands::{Frames, DIMS};
use crate::structure::{BEATS_PER_BAR, PHRASE_BARS};
use crate::Analysis;
use serde::{Deserialize, Serialize};
use stelly_audio::decode::Pcm;

/// Bump when what counts as a drop changes, so drops an older build kept
/// are found again rather than shown.
pub const VERSION: u32 = 4;

/// Bands 0..4 run from 30 to about 145Hz: the kick's body and the sub.
const LOW_BANDS: usize = 4;
/// Bars either side of a bar line.
const WIDTH: usize = 4;
/// How far the low end must climb, dB.
const JUMP_DB: f64 = 6.0;
/// How far under its usual level it must have been, and how near it it
/// must come back to, dB.
const BACK_DB: f64 = 3.0;

/// The drops found in a track, and by which version of finding them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drops {
    pub version: u32,
    /// Seconds.
    pub at: Vec<f64>,
}

impl Drops {
    pub fn new(at: Vec<f64>) -> Self {
        Self { version: VERSION, at }
    }

    /// Whether these were found the way this build finds them.
    pub fn current(&self) -> bool {
        self.version == VERSION
    }
}

/// Where the drops land, seconds, in a track's `frames` counted on `beats`,
/// the first downbeat being `beats[downbeat]`. Found by the bar, then by the
/// beat where the low end steps up, then in the audio itself: the bars are
/// only as right as the downbeat, and the beats as right as the grid's
/// phase, which can be a fraction of a beat out and leave the drop between
/// two of them.
pub fn at(frames: &Frames, beats: &[f64], downbeat: usize) -> Vec<f64> {
    let rows = frames.per_beat(beats);
    let low = low_per_beat(&rows);
    let heard = low_per_beat(&frames.rows);
    find(&low_per_bar(&rows, downbeat))
        .into_iter()
        .filter_map(|bar| {
            let beat = landing(&low, downbeat + bar * BEATS_PER_BAR);
            let period = beats.get(beat + 1)? - beats.get(beat)?;
            Some(onset(frames, &heard, beats[beat], period))
        })
        .collect()
}

/// Where within a beat of `beat` the low end jumps, `heard` frame by frame:
/// the frame where the half beat from it is loudest against the whole beat
/// before it, the first of equals. A whole beat, so that the kick after the
/// one that drops, with a half beat of quiet before it too, does not tie.
fn onset(frames: &Frames, heard: &[f64], beat: f64, period: f64) -> f64 {
    let whole = ((period / frames.hop()).round() as usize).max(2);
    let half = whole / 2;
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    let step = |f: usize| mean(&heard[f..f + half]) - mean(&heard[f - whole..f]);
    (whole..heard.len().saturating_sub(half))
        .filter(|&f| (frames.centre(f) - beat).abs() <= period)
        .map(|f| (f, step(f)))
        .fold(None, |best: Option<(usize, f64)>, (f, s)| match best {
            Some((_, b)) if b >= s => best,
            _ => Some((f, s)),
        })
        .map_or(beat, |(f, _)| frames.centre(f))
}

/// The beat within a bar either side of `bar_line` where the low end climbs
/// most, a bar before against a bar from it.
fn landing(low: &[f64], bar_line: usize) -> usize {
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    let step = |b: usize| mean(&low[b..b + BEATS_PER_BAR]) - mean(&low[b - BEATS_PER_BAR..b]);
    (bar_line.saturating_sub(BEATS_PER_BAR - 1)..=bar_line + BEATS_PER_BAR - 1)
        .filter(|&b| b >= BEATS_PER_BAR && b + BEATS_PER_BAR <= low.len())
        .max_by(|&a, &b| step(a).total_cmp(&step(b)))
        .unwrap_or(bar_line)
}

/// The drops of a track analysed before they were looked for, or by an
/// older build, from its audio and the beats it was analysed with.
pub fn of(pcm: &Pcm, analysis: &Analysis) -> Drops {
    let frames = Frames::of(&pcm.samples, pcm.rate);
    let downbeat = analysis
        .downbeats
        .first()
        .and_then(|d| analysis.beats.iter().position(|b| (b - d).abs() < 1e-6))
        .unwrap_or(0);
    Drops::new(at(&frames, &analysis.beats, downbeat))
}

/// The low end of each row, beat or frame, dB.
fn low_per_beat(rows: &[[f64; DIMS]]) -> Vec<f64> {
    rows.iter().map(|b| decibels(b[..LOW_BANDS].iter().map(|v| v.exp()).sum())).collect()
}

fn decibels(energy: f64) -> f64 {
    10.0 * energy.max(1e-10).log10()
}

/// The low end of each bar from the first downbeat, dB from the track's
/// median bar.
pub fn low_per_bar(rows: &[[f64; DIMS]], downbeat: usize) -> Vec<f64> {
    let mut bars: Vec<f64> = rows[downbeat.min(rows.len())..]
        .chunks_exact(BEATS_PER_BAR)
        .map(|bar| decibels(bar.iter().map(|b| b[..LOW_BANDS].iter().map(|v| v.exp()).sum::<f64>()).sum::<f64>() / bar.len() as f64))
        .collect();
    let mut sorted = bars.clone();
    sorted.sort_by(f64::total_cmp);
    if let Some(&median) = sorted.get(sorted.len() / 2) {
        bars.iter_mut().for_each(|b| *b -= median);
    }
    bars
}

/// Bars, counted like `low`, where a drop lands: the steepest climb within
/// a phrase either way, so a drop counts once.
pub fn find(low: &[f64]) -> Vec<usize> {
    let rise: Vec<Option<f64>> = (0..low.len()).map(|k| rise(low, k)).collect();
    let reach = PHRASE_BARS / 2;
    (0..rise.len())
        .filter(|&k| {
            let Some(v) = rise[k] else { return false };
            v >= JUMP_DB
                && (k.saturating_sub(reach)..(k + reach + 1).min(rise.len()))
                    .filter(|&j| j != k)
                    .all(|j| rise[j].is_none_or(|u| if j < k { u < v } else { u <= v }))
        })
        .collect()
}

/// How much the low end climbs from the bars before `at` to the bars from
/// it, when every bar before is in a dip and every bar from it is back near
/// the usual level. Every bar, not on average: a groove that takes the bass
/// out of every other bar averages to a dip, and a breakdown that brings
/// half the low end back averages to a return. Not in the first phrase,
/// where it is the drums coming in after a short intro.
fn rise(low: &[f64], at: usize) -> Option<f64> {
    if at < PHRASE_BARS || at + WIDTH > low.len() {
        return None;
    }
    let (before, after) = (&low[at - WIDTH..at], &low[at..at + WIDTH]);
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
    (before.iter().all(|&b| b <= -BACK_DB) && after.iter().all(|&a| a >= -BACK_DB)).then(|| mean(after) - mean(before))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beats::tests::{kick, RATE};

    #[test]
    fn a_drop_after_a_breakdown() {
        let low: Vec<f64> = [vec![0.0; 16], vec![-12.0; 8], vec![0.0; 16]].concat();
        assert_eq!(find(&low), vec![24]);
    }

    #[test]
    fn not_the_drums_coming_in_after_an_intro() {
        let low: Vec<f64> = [vec![-12.0; 4], vec![0.0; 30]].concat();
        assert_eq!(find(&low), Vec::<usize>::new());
    }

    #[test]
    fn not_the_last_bar_of_a_build() {
        let low: Vec<f64> = [vec![0.0; 16], vec![-12.0; 7], vec![-5.0], vec![0.0; 16]].concat();
        assert_eq!(find(&low), vec![24]);
    }

    #[test]
    fn not_loud_to_louder() {
        let low: Vec<f64> = [vec![-2.0; 16], vec![6.0; 16]].concat();
        assert_eq!(find(&low), Vec::<usize>::new());
    }

    /// Kicks on every beat but for an eight-bar breakdown of hats alone.
    #[test]
    fn heard_in_audio() {
        let (bpm, bars, breakdown) = (128.0, 40, 16..24);
        let period = 60.0 / bpm;
        let beats: Vec<f64> = (0..=bars * BEATS_PER_BAR).map(|b| 0.1 + b as f64 * period).collect();
        let mut audio = vec![0f32; ((beats.last().unwrap() + 1.0) * RATE as f64) as usize];
        let mut noise = 1u32;
        for (b, &t) in beats.iter().enumerate() {
            let at = (t * RATE as f64) as usize;
            if !breakdown.contains(&(b / BEATS_PER_BAR)) {
                kick(&mut audio, at, 0.6);
            }
            for i in 0..2_000 {
                noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if let Some(slot) = audio.get_mut(at + RATE as usize / 4 + i) {
                    *slot += 0.05 * (noise as f32 / u32::MAX as f32 - 0.5);
                }
            }
        }
        let found = at(&Frames::of(&audio, RATE), &beats, 0);
        assert!(near(&found, beats[breakdown.end * BEATS_PER_BAR]), "{found:?}");
    }

    /// Counted from a downbeat two beats out, the drop is still on the beat
    /// the kicks come back on, not the bar line after it.
    #[test]
    fn on_the_beat_with_the_bars_out() {
        let (bpm, bars, back) = (128.0, 40, 24 * BEATS_PER_BAR);
        let period = 60.0 / bpm;
        let beats: Vec<f64> = (0..=bars * BEATS_PER_BAR).map(|b| 0.1 + b as f64 * period).collect();
        let mut audio = vec![0f32; ((beats.last().unwrap() + 1.0) * RATE as f64) as usize];
        for (b, &t) in beats.iter().enumerate() {
            if !(16 * BEATS_PER_BAR..back).contains(&b) {
                kick(&mut audio, (t * RATE as f64) as usize, 0.6);
            }
        }
        let found = at(&Frames::of(&audio, RATE), &beats, 2);
        assert!(near(&found, beats[back]), "{found:?}");
    }

    /// With the grid's phase out by most of a beat, the drop is where the
    /// kicks come back, between two of its beats.
    #[test]
    fn in_the_audio_with_the_beats_out() {
        let (bpm, bars, back) = (128.0, 40, 24 * BEATS_PER_BAR);
        let period = 60.0 / bpm;
        let kicks: Vec<f64> = (0..=bars * BEATS_PER_BAR).map(|b| 0.5 + b as f64 * period).collect();
        let mut audio = vec![0f32; ((kicks.last().unwrap() + 1.0) * RATE as f64) as usize];
        for (b, &t) in kicks.iter().enumerate() {
            if !(16 * BEATS_PER_BAR..back).contains(&b) {
                kick(&mut audio, (t * RATE as f64) as usize, 0.6);
            }
        }
        let beats: Vec<f64> = kicks.iter().map(|k| k + 0.6 * period).collect();
        let found = at(&Frames::of(&audio, RATE), &beats, 0);
        assert!(near(&found, kicks[back]), "{found:?}, kicks back at {:.3}", kicks[back]);
    }

    /// One drop, within a frame or two of `expected`.
    fn near(found: &[f64], expected: f64) -> bool {
        found.len() == 1 && (found[0] - expected).abs() < 0.05
    }
}
