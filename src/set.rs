//! Planning a set: a chain of transitions from a first track, no track twice,
//! as long as asked for.
//!
//! The set keeps the first track's tempo, as many DJs do, so every track in
//! it is ranked by its stretch from that tempo, not from the one before:
//! otherwise a run of 155s scores as perfect while each is played 3% fast. Greedy picking takes the best next step and can walk into a
//! corner where every way on is poor, so a beam of the best partial sets is
//! carried forward, each grown by its best few next steps, and the set with
//! the best average transition wins.

use crate::library::Track;
use crate::transitions::{self, fold, same_recording, same_song, Transition};

#[derive(Debug, Clone)]
pub struct Options {
    /// Seconds of music the set should reach.
    pub seconds: f64,
    /// Largest tempo change from the set's tempo, as a fraction.
    pub max_stretch: f64,
    /// Partial sets carried forward.
    pub beam: usize,
    /// Next steps tried from each.
    pub branch: usize,
    /// Least set seconds a track plays, from its transition in to the start
    /// of its transition out: without it a track can come in and go out
    /// again a phrase later, barely heard.
    pub min_play: f64,
    /// How much each library track sounds like what the set should, 0..1,
    /// from `text::scores`: tracks under `LIKE_FLOOR` are left out, and the
    /// rest count towards a transition's score.
    pub like: Option<Vec<f64>>,
    /// A transition scoring under this is not taken, however good the rest
    /// of the set: the plan maximises the average, which one bad transition
    /// can hide in.
    pub min_transition: f64,
    /// Tracks the plan may not use, by their place in the library: what a
    /// set already holds before the part being planned again.
    pub avoid: Vec<usize>,
}



/// The default `min_transition`: with sound scored against each track's
/// closest matches, a sound that is merely typical of them scores under 0.5,
/// and a floor there stopped half the sets tried after a few tracks.
pub const MIN_TRANSITION: f64 = 0.35;

/// Rank under which a track is too far from the set's description to play.
const LIKE_FLOOR: f64 = 0.6;
/// How much sounding like the description counts, against a transition's
/// own score, which is 0..1.
const LIKE_WEIGHT: f64 = 0.3;

impl Default for Options {
    fn default() -> Self {
        Self {
            seconds: 30.0 * 60.0,
            max_stretch: 0.06,
            beam: 8,
            branch: 6,
            min_play: 90.0,
            like: None,
            min_transition: MIN_TRANSITION,
            avoid: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// Indices into the library, in playing order.
    pub tracks: Vec<usize>,
    /// `transitions[k]` takes `tracks[k]` into `tracks[k + 1]`; its `to` is
    /// an index into the library.
    pub transitions: Vec<Transition>,
    /// Set seconds up to the last transition.
    pub seconds: f64,
    pub score: f64,
}

impl Plan {
    pub fn mean_score(&self) -> f64 {
        self.score / self.transitions.len().max(1) as f64
    }
}

/// The best set from `library[first]` that reaches `options.seconds`, or the
/// longest one there is if the library runs out first.
pub fn plan(library: &[Track], first: usize, options: &Options) -> Option<Plan> {
    let master = library[first].bpm()?;
    let rate = |track: &Track| track.bpm().map_or(1.0, |bpm| fold(bpm / master));
    let ranking = transitions::Options {
        max_stretch: options.max_stretch,
        tempo: Some(master),
        sound: transitions::SoundScale::of(library),
    };

    let mut beam = vec![Plan {
        tracks: vec![first],
        transitions: Vec::new(),
        seconds: 0.0,
        score: 0.0,
    }];
    let mut finished: Vec<Plan> = Vec::new();
    while !beam.is_empty() {
        let mut grown = Vec::new();
        for plan in &beam {
            let last = *plan.tracks.last().expect("a plan has a first track");
            let playing = &library[last];
            // Where the playing track came in, in its own seconds.
            let entered = plan.transitions.last().map_or(0.0, |t| playing.segments[t.into].start);
            let liked = |i: usize| options.like.as_ref().map_or(1.0, |like| like[i]);
            let steps = transitions::from_allowed(playing, library, &ranking, |i, next| {
                // Nor another edit of a song already in it.
                let played = plan.tracks.iter().chain(&options.avoid).any(|&p| p == i || same_recording(&library[p], next) || same_song(&library[p], next));
                !played && steady(next) && liked(i) >= LIKE_FLOOR
            });
            let mut extended = false;
            let long_enough = |step: &Transition| (playing.segments[step.out].start - entered) * rate(playing) >= options.min_play;
            let good = |step: &Transition| step.score >= options.min_transition;
            for step in steps.into_iter().filter(long_enough).filter(good).take(options.branch) {
                let leaves = playing.segments[step.out].start;
                let mut next = plan.clone();
                next.seconds += (leaves - entered) * rate(playing);
                next.score += step.score + options.like.as_ref().map_or(0.0, |_| LIKE_WEIGHT * liked(step.to));
                next.tracks.push(step.to);
                next.transitions.push(step);
                extended = true;
                if next.seconds >= options.seconds {
                    finished.push(next);
                } else {
                    grown.push(next);
                }
            }
            if !extended {
                finished.push(plan.clone());
            }
        }
        grown.sort_by(|a, b| b.mean_score().total_cmp(&a.mean_score()));
        grown.truncate(options.beam);
        beam = grown;
    }

    // Long enough first, then the best transitions.
    let long_enough = finished.iter().any(|p| p.seconds >= options.seconds);
    finished
        .into_iter()
        .filter(|p| !long_enough || p.seconds >= options.seconds)
        .max_by(|a, b| {
            if long_enough {
                a.mean_score().total_cmp(&b.mean_score())
            } else {
                a.seconds.total_cmp(&b.seconds)
            }
        })
}

/// Whether a track keeps one tempo, so a set can play it at one rate and
/// stay on the beat. One that drifts, a live drummer, a rubato, drifts off
/// the beat of whatever it is mixed with: a set leaves it out, as it already
/// would not start on one.
pub fn steady(track: &Track) -> bool {
    track.analysis.grid.as_ref().is_some_and(|g| g.steady)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transitions::tests::track;

    #[test]
    fn a_set_leaves_out_a_track_whose_tempo_drifts() {
        let phrases = [0, 1, 2, 3, 4, 5, 6, 7];
        let mut drifting = track("Artist - Song [101]", &phrases, false);
        drifting.analysis.grid.as_mut().unwrap().steady = false;
        let library = vec![track("Artist - Outro", &phrases, false), drifting, track("Artist - Closer", &phrases, false)];
        let plan = plan(&library, 0, &Options { seconds: 6.0 * 60.0 * 60.0, min_play: 10.0, ..Default::default() }).expect("a set");
        assert_eq!(plan.tracks, vec![0, 2], "the steady ones, and not the one that drifts");
    }

    #[test]
    fn a_set_never_plays_two_edits_of_one_song() {
        // Named as local files are, by their file; all of them mix with all.
        // The test tracks all sound alike, so only their lengths, which
        // their names set, keep them from passing for copies of one
        // recording: these are each 3s or more from the rest.
        let phrases = [0, 1, 2, 3, 4, 5, 6, 7];
        let library = vec![
            track("Artist - Outro", &phrases, false),
            track("Artist - Song [101]", &phrases, false),
            track("Artist - Song (Radio Edit) [102]", &phrases, false),
            track("Artist - Closer", &phrases, false),
        ];
        let plan = plan(&library, 0, &Options { seconds: 6.0 * 60.0 * 60.0, min_play: 10.0, ..Default::default() }).expect("a set");
        assert!(plan.tracks.len() >= 3, "the set still runs through what it can: {:?}", plan.tracks);
        assert!(!(plan.tracks.contains(&1) && plan.tracks.contains(&2)), "the song twice: {:?}", plan.tracks);
    }
}
