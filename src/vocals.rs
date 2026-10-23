//! Whether there is a voice in it, phrase by phrase.
//!
//! Two tracks with someone singing over each other is the transition nobody
//! wants: the ear follows a voice, and two of them at once is not a mix, it
//! is a mistake. So this is worth knowing twice over — on a track's line,
//! before it is chosen, and at the phrase the mix cuts on, so that cut can
//! be moved somewhere the singing is not.
//!
//! No new model and nothing re-analysed: every phrase already keeps CLAP's
//! audio embedding, and CLAP's text tower puts "singing" in the same space.
//! A phrase is scored by how much nearer it is to a voice than to an
//! instrumental — the two asked for together, because CLAP has no notion of
//! "not": asking it for "no human voice" scores an orchestra higher than a
//! singer, which is how the overlap was found.
//!
//! It is a guess, and a narrow one. Measured over tracks whose answer is not
//! in doubt, singing scored -0.049 and up while instrumentals reached
//! -0.115, so the line sits between them with little room either side. A
//! dense, distorted mix hides its voice from CLAP: the loudest false reading
//! found was a nu-metal track with shouted vocals throughout, which landed
//! just over the line. Anything built on this should say that it is
//! guessing, and should prefer the score to the verdict.

use crate::library::Track;
use crate::segments::Segment;
use crate::text::TextEncoder;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// What a voice is asked for as.
pub const VOCAL: &str = "a song with singing, a voice, vocals, lyrics";
/// And what it is weighed against. Not a negation: see the module's note.
pub const INSTRUMENTAL: &str = "an instrumental piece, no singing, no voice";

/// Over this, a phrase is taken to have a voice in it. Halfway between the
/// two sides of the measurement above.
pub const SPEAKS: f64 = -0.08;

/// Bump when the prompts or the scoring change, so scores worked out by an
/// older build are redone rather than mixed with these.
pub const VERSION: u32 = 1;

/// What was heard in a track, kept so the text tower is loaded once and not
/// again on every run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heard {
    pub version: u32,
    /// The track's own score: the mean of its phrases.
    pub track: f64,
    /// One a phrase, in the same order as `Track::segments`.
    pub phrases: Vec<f64>,
}

impl Heard {
    /// Whether to say, on a line, that there is singing in it.
    pub fn speaks(&self) -> bool {
        self.track > SPEAKS
    }

    /// Whether the phrase at `at` has a voice in it. A phrase that was never
    /// scored is taken as not: a guess should not stop a mix.
    pub fn phrase_speaks(&self, at: usize) -> bool {
        self.phrases.get(at).is_some_and(|&score| score > SPEAKS)
    }

    /// How much of a voice is in the phrase at `at`, 0 at the line and 1 well
    /// over it, for weighing rather than deciding.
    pub fn how_much(&self, at: usize) -> f64 {
        self.phrases.get(at).map_or(0.0, |&score| ((score - SPEAKS) / SPREAD).clamp(0.0, 1.0))
    }
}

/// The range over the line that counts as fully sung, from the measurement:
/// the clearest singing sat about a quarter above it.
const SPREAD: f64 = 0.25;

/// The two prompts, embedded, ready to score against.
pub struct Ear {
    vocal: Vec<f32>,
    instrumental: Vec<f32>,
}

impl Ear {
    /// The text tower, fetched if it is not here yet. Half a gigabyte, so a
    /// caller that scores a whole library should hold on to this.
    pub fn load() -> Result<Self> {
        let models = crate::models::dir();
        crate::models::ensure_text(&models)?;
        let mut tower = TextEncoder::load(&models)?;
        Self::of(&mut tower)
    }

    /// The same from a tower already loaded, for whoever has one.
    pub fn of(tower: &mut TextEncoder) -> Result<Self> {
        Ok(Self { vocal: tower.embed(VOCAL)?, instrumental: tower.embed(INSTRUMENTAL)? })
    }

    pub fn segment(&self, segment: &Segment) -> f64 {
        dot(&segment.embedding, &self.vocal) - dot(&segment.embedding, &self.instrumental)
    }

    /// Every phrase of `track`, and the track itself.
    pub fn track(&self, track: &Track) -> Heard {
        let phrases: Vec<f64> = track.segments.iter().map(|segment| self.segment(segment)).collect();
        let mean = if phrases.is_empty() { 0.0 } else { phrases.iter().sum::<f64>() / phrases.len() as f64 };
        Heard { version: VERSION, track: mean, phrases }
    }
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x * y) as f64).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(phrases: Vec<f64>) -> Heard {
        let track = phrases.iter().sum::<f64>() / phrases.len().max(1) as f64;
        Heard { version: VERSION, track, phrases }
    }

    /// The quietest singing found over the tracks the prompts were chosen
    /// on: a nu-metal track whose shouted vocals CLAP nearly loses in the
    /// distortion.
    const QUIETEST_SINGING: f64 = -0.049;
    /// And the most vocal-sounding instrumental among them.
    const MOST_VOCAL_INSTRUMENTAL: f64 = -0.115;

    /// The line has to sit between the two, with room either side. Moving
    /// it past something that was measured should fail to build rather than
    /// quietly mislabel half a library.
    const _: () = assert!(
        SPEAKS - MOST_VOCAL_INSTRUMENTAL > 0.02 && QUIETEST_SINGING - SPEAKS > 0.02,
        "SPEAKS has moved past one of the tracks it was measured against"
    );

    #[test]
    fn a_track_speaks_when_its_phrases_do() {
        assert!(heard(vec![0.1, 0.2, 0.05]).speaks());
        assert!(!heard(vec![-0.3, -0.25, -0.2]).speaks());
        // An instrumental with one sung phrase is not a song.
        assert!(!heard(vec![-0.3, -0.3, -0.3, 0.2]).speaks());
    }

    #[test]
    fn a_phrase_is_judged_on_its_own() {
        let it = heard(vec![-0.3, 0.1, -0.3]);
        assert!(!it.phrase_speaks(0));
        assert!(it.phrase_speaks(1), "the sung phrase, in an otherwise quiet track");
        assert!(!it.phrase_speaks(9), "a phrase that was never scored does not stop a mix");
    }

    #[test]
    fn how_much_runs_from_the_line_to_well_over_it() {
        let it = heard(vec![-0.5, SPEAKS, SPEAKS + SPREAD / 2.0, SPEAKS + SPREAD, 1.0]);
        assert_eq!(it.how_much(0), 0.0, "well under the line is none of it");
        assert_eq!(it.how_much(1), 0.0, "at the line is still none");
        assert!((it.how_much(2) - 0.5).abs() < 1e-9);
        assert_eq!(it.how_much(3), 1.0);
        assert_eq!(it.how_much(4), 1.0, "and it does not run past 1");
        assert_eq!(it.how_much(9), 0.0, "nor does a phrase nobody scored");
    }
}
