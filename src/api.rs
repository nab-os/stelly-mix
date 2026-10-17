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

/// What to have a server analyse for mixing. On the command line and in the
/// app's Index box it is written as a reference, see `FromStr`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "pick", rename_all = "kebab-case")]
pub enum Selection {
    /// One track, its album catalogued with it.
    Track { id: i64 },
    Album { id: String },
    /// The artist's own releases.
    Artist { id: i64 },
    /// A Qobuz playlist's tracks.
    Playlist { id: i64 },
    /// What the family has liked: tracks, and albums whole.
    Favourites,
    /// A track and its nearest `count` in the server's space.
    Around { seed: i64, count: usize },
    /// Every track in the space credited to `name`, and the nearest `count`
    /// of each.
    ArtistName { name: String, count: usize },
    /// The `count` tracks in the space nearest a description.
    Mood { phrase: String, count: usize },
    /// The space's tracks whose genre holds any of `genres`, at most `limit`,
    /// 0 for all.
    Space { genres: Vec<String>, limit: usize },
}

/// How many nearest to each, unless a reference says `?n=`.
pub const AROUND: usize = 20;
/// How many for a mood, unless `?n=`.
pub const MOOD: usize = 100;

impl std::str::FromStr for Selection {
    type Err = String;

    /// `track/<id>`, `album/<id>`, `artist/<id>`, `playlist/<id>`,
    /// `favourites`, `around/<id>`, `artist-name/<name>`, `mood/<words>` and
    /// `genre/<genre>,<genre>`, the last four with an optional `?n=<count>`;
    /// with or without a leading `stelly:`.
    fn from_str(reference: &str) -> Result<Self, String> {
        let reference = reference.trim();
        let reference = reference.strip_prefix("stelly:").unwrap_or(reference);
        let (what, count) = match reference.rsplit_once("?n=") {
            Some((what, n)) => (what, Some(n.parse::<usize>().map_err(|_| format!("?n={n} is not a count"))?)),
            None => (reference, None),
        };
        let id = |text: &str| text.trim().parse::<i64>().map_err(|_| format!("{text:?} is not an id"));
        let text = |text: &str| {
            let text = text.trim();
            if text.is_empty() { Err(format!("{reference}: what after the slash?")) } else { Ok(text.to_string()) }
        };
        Ok(match what.split_once('/') {
            None if what == "favourites" || what == "favorites" => Selection::Favourites,
            Some(("track", rest)) => Selection::Track { id: id(rest)? },
            Some(("album", rest)) => Selection::Album { id: text(rest)? },
            Some(("artist", rest)) => Selection::Artist { id: id(rest)? },
            Some(("playlist", rest)) => Selection::Playlist { id: id(rest)? },
            Some(("around", rest)) => Selection::Around { seed: id(rest)?, count: count.unwrap_or(AROUND) },
            Some(("artist-name", rest)) => Selection::ArtistName { name: text(rest)?, count: count.unwrap_or(AROUND) },
            Some(("mood", rest)) => Selection::Mood { phrase: text(rest)?, count: count.unwrap_or(MOOD) },
            Some(("genre", rest)) => Selection::Space {
                genres: rest.split(',').map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect(),
                limit: count.unwrap_or(0),
            },
            _ => {
                return Err(format!(
                    "{reference}: expected track/<id>, album/<id>, artist/<id>, playlist/<id>, favourites, \
                     around/<id>, artist-name/<name>, mood/<words> or genre/<genres>"
                ))
            }
        })
    }
}

impl std::fmt::Display for Selection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Selection::Track { id } => write!(f, "track {id}"),
            Selection::Album { id } => write!(f, "album {id}"),
            Selection::Artist { id } => write!(f, "artist {id}"),
            Selection::Playlist { id } => write!(f, "playlist {id}"),
            Selection::Favourites => write!(f, "the family's likes"),
            Selection::Around { seed, count } => write!(f, "track {seed} and its {count} nearest"),
            Selection::ArtistName { name, count } => write!(f, "{name}'s tracks and their {count} nearest"),
            Selection::Mood { phrase, count } => write!(f, "the {count} tracks nearest \"{phrase}\""),
            Selection::Space { genres, limit: 0 } => write!(f, "the space's {}", genres.join(", ")),
            Selection::Space { genres, limit } => write!(f, "the space's {} (at most {limit})", genres.join(", ")),
        }
    }
}

/// How an analysis went.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
}

/// What a server's background analysis is doing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JobStatus {
    pub running: bool,
    pub selection: Option<String>,
    /// Its last line: a track done, or why it stopped.
    pub last: Option<String>,
    /// How the last run went, once it has.
    pub stats: Option<Stats>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_read_as_selections() {
        let read = |r: &str| r.parse::<Selection>();
        assert_eq!(read("stelly:album/0060254728690"), Ok(Selection::Album { id: "0060254728690".into() }));
        assert_eq!(read("track/42"), Ok(Selection::Track { id: 42 }));
        assert_eq!(read("playlist/7"), Ok(Selection::Playlist { id: 7 }));
        assert_eq!(read("favourites"), Ok(Selection::Favourites));
        assert_eq!(read("around/42?n=50"), Ok(Selection::Around { seed: 42, count: 50 }));
        assert_eq!(read("artist-name/Lun"), Ok(Selection::ArtistName { name: "Lun".into(), count: AROUND }));
        assert_eq!(read("mood/dark rolling techno?n=30"), Ok(Selection::Mood { phrase: "dark rolling techno".into(), count: 30 }));
        assert_eq!(read("genre/techno, house?n=500"), Ok(Selection::Space { genres: vec!["techno".into(), "house".into()], limit: 500 }));
        for bad in ["album/", "track/x", "around/1?n=many", "everything", "mood/"] {
            assert!(read(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn selections_cross_the_wire_by_name() {
        let json = serde_json::to_value(Selection::Around { seed: 42, count: 20 }).unwrap();
        assert_eq!(json, serde_json::json!({ "pick": "around", "seed": 42, "count": 20 }));
        assert_eq!(serde_json::to_value(Selection::Favourites).unwrap(), serde_json::json!({ "pick": "favourites" }));
        let back: Selection = serde_json::from_value(serde_json::json!({ "pick": "artist-name", "name": "Lun", "count": 5 })).unwrap();
        assert_eq!(back, Selection::ArtistName { name: "Lun".into(), count: 5 });
    }
}
