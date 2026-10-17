//! What crosses between a server holding a mix library, Stelly's, and its
//! clients: the app and the command line. Tracks are named by their Qobuz id,
//! which is what the server's catalogue is keyed by.

use crate::library::Track;
use crate::transitions::Transition;
use serde::{Deserialize, Serialize};

/// A library track, as a list shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSummary {
    pub id: i64,
    pub artist: String,
    pub title: String,
    pub bpm: Option<f64>,
    /// Made on a grid: its beats are the grid's.
    pub steady: bool,
    /// Camelot, "8A".
    pub key: Option<String>,
    pub seconds: f64,
    pub phrases: usize,
}

impl TrackSummary {
    pub fn of(id: i64, track: &Track) -> Self {
        let (artist, title) = match &track.origin {
            Some(origin) => (origin.item.artist.clone(), origin.item.title.clone()),
            None => (String::new(), track.name()),
        };
        Self {
            id,
            artist,
            title,
            bpm: track.bpm(),
            steady: track.analysis.grid.as_ref().is_some_and(|g| g.steady),
            key: track.analysis.key.as_ref().map(|k| k.camelot.clone()),
            seconds: track.analysis.seconds,
            phrases: track.analysis.phrases.len(),
        }
    }

    pub fn name(&self) -> String {
        if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} - {}", self.artist, self.title)
        }
    }
}

/// A way from one track into another: where, and how good.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Way {
    /// The track mixed into.
    pub to: i64,
    pub score: f64,
    pub sound: f64,
    /// Incoming tempo over the one it will play at, octaves folded.
    pub ratio: f64,
    pub key_steps: Option<u32>,
    /// Incoming loudness minus outgoing, LU.
    pub energy: Option<f64>,
    /// The segment mixed out of, in the playing track, and its seconds.
    pub out: usize,
    pub out_at: (f64, f64),
    /// The segment mixed into, in the next track, and its seconds.
    pub into: usize,
    pub into_at: (f64, f64),
}

impl Way {
    /// `transition` from `playing` into `next`, known to the client as `to`.
    pub fn of(transition: &Transition, playing: &Track, next: &Track, to: i64) -> Self {
        let (out, into) = (&playing.segments[transition.out], &next.segments[transition.into]);
        Self {
            to,
            score: transition.score,
            sound: transition.sound,
            ratio: transition.ratio,
            key_steps: transition.key_steps,
            energy: transition.energy,
            out: transition.out,
            out_at: (out.start, out.end),
            into: transition.into,
            into_at: (into.start, into.end),
        }
    }

    /// Back as the renderer takes it, the next track at `to` in its chain.
    pub fn transition(&self, to: usize) -> Transition {
        Transition {
            to,
            out: self.out,
            into: self.into,
            score: self.score,
            sound: self.sound,
            ratio: self.ratio,
            key_steps: self.key_steps,
            energy: self.energy,
        }
    }
}

/// What a set is to be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetRequest {
    /// The track to start from; without one, the best match for `like`.
    pub first: Option<i64>,
    /// What the set should sound like.
    pub like: Option<String>,
    pub minutes: f64,
    /// Largest tempo change from the first track's, percent.
    pub stretch: f64,
    pub beam: usize,
    pub min_transition: f64,
}

impl Default for SetRequest {
    fn default() -> Self {
        let defaults = crate::set::Options::default();
        Self {
            first: None,
            like: None,
            minutes: defaults.seconds / 60.0,
            stretch: defaults.max_stretch * 100.0,
            beam: defaults.beam,
            min_transition: defaults.min_transition,
        }
    }
}

/// A set planned: its tracks in order, and the way from each into the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPlan {
    pub tracks: Vec<i64>,
    pub ways: Vec<Way>,
    /// Set seconds up to the last transition.
    pub seconds: f64,
    pub mean_score: f64,
}

/// How a library matches a description, best first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Match {
    pub id: i64,
    /// Its rank across the library, 0..1.
    pub score: f64,
}

/// What a server's mix library holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub tracks: usize,
    /// What the analysis version is, so a client knows what it reads.
    pub version: u32,
}

/// A library entry with its Qobuz id, as a server hands several over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: i64,
    pub track: Track,
}

/// Which entries to hand over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntriesRequest {
    pub ids: Vec<i64>,
}
