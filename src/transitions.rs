//! Where to go next: from a phrase late in the playing track into a phrase
//! early in another, scored on what a DJ weighs.
//!
//! - **Sound**: CLAP cosine between the two phrases, so a transition joins
//!   passages that sound alike rather than tracks that do on average. Within
//!   a genre every pair is 0.93–0.96, so a best pair is scored against what
//!   a good match is in this library, `SoundScale`: each track's closest few
//!   others, two deviations above their mean scoring 1. Not every pair: in a
//!   library of many genres most pairs are far apart, and against them every
//!   passable match would look excellent and the key would choose instead.
//!   Measured once over the library, not over whichever candidates a step
//!   allows, so the same pair scores the same in every step of a set.
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
    /// The tempo the incoming track will play at, when it is not the playing
    /// track's own: a set held at one tempo. Stretch is measured from it.
    pub tempo: Option<f64>,
    /// What a best pair's sound is scored against; `SoundScale::of` the
    /// library, ideally.
    pub sound: SoundScale,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_stretch: 0.06,
            tempo: None,
            sound: SoundScale::default(),
        }
    }
}

/// Where best pairs' sounds sit across a library: their mean and spread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundScale {
    pub mean: f64,
    pub spread: f64,
}

/// Tracks whose closest matches set a library's scale, at most: enough for
/// a steady mean, few enough to take well under a second.
const SCALE_TRACKS: usize = 60;
/// How many of each one's closest matches in the library count: the
/// candidates a good next track is chosen among.
const SCALE_CLOSEST: usize = 10;
/// Tracks each is compared with, at most: past this a fixed spread of the
/// library stands for it, and its closest few for the library's closest ten.
const SCALE_SAMPLE: usize = 3000;

impl Default for SoundScale {
    /// Within a genre every pair is 0.93–0.96: what is used without a
    /// library to measure.
    fn default() -> Self {
        Self {
            mean: 0.945,
            spread: 0.0075,
        }
    }
}

impl SoundScale {
    /// Measured over `tracks`: for up to `SCALE_TRACKS` of them, spread
    /// through the library, the `SCALE_CLOSEST` best pairs with any other
    /// track, copies of the same recording left out. The same tracks every
    /// time for the same library.
    pub fn of(tracks: &[Track]) -> Self {
        let usable: Vec<&Track> = tracks.iter().filter(|t| !t.segments.is_empty()).collect();
        let n = usable.len();
        if n < 3 {
            return Self::default();
        }
        let sample: Vec<&Track> = if n <= SCALE_SAMPLE {
            usable.clone()
        } else {
            (0..SCALE_SAMPLE).map(|i| usable[i * n / SCALE_SAMPLE]).collect()
        };
        // The library's closest ten are, in a sample of it, its closest few.
        let closest = ((SCALE_CLOSEST * sample.len()) as f64 / n as f64).round().max(1.0) as usize;
        let closest = closest.min(sample.len().saturating_sub(1)).max(1);
        let measured = SCALE_TRACKS.min(n);
        let nearest = |i: usize| -> Vec<f64> {
            let a = usable[i * n / measured];
            let mut near: Vec<f64> = sample
                .iter()
                .filter(|b| b.path != a.path && !same_recording(a, b))
                .filter_map(|b| best_sound(a, b))
                .collect();
            near.sort_by(|x, y| y.total_cmp(x));
            near.truncate(closest);
            near
        };
        // Each track against the whole library: spread over the cores.
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(measured);
        let sounds: Vec<f64> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|t| scope.spawn(move || (t..measured).step_by(threads).flat_map(nearest).collect::<Vec<f64>>()))
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("a scale thread panicked")).collect()
        });
        if sounds.len() < 3 {
            return Self::default();
        }
        let count = sounds.len() as f64;
        let mean = sounds.iter().sum::<f64>() / count;
        let spread = (sounds.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / count).sqrt();
        Self {
            mean,
            // A library of near-copies would make every difference a chasm.
            spread: spread.max(0.002),
        }
    }

    /// 0..1: two spreads under the mean is a 0, two over a 1.
    pub fn score(&self, sound: f64) -> f64 {
        (((sound - self.mean) / self.spread + 2.0) / 4.0).clamp(0.0, 1.0)
    }
}

/// Whether two library entries are the same recording: a FLAC and an MP3 of
/// it, a copy in a playlist folder, a rip of the video. Their lengths agree
/// to a couple of seconds, and either their names do once track numbers and
/// punctuation are set aside, or their audio does, phrase for phrase.
pub fn same_recording(a: &Track, b: &Track) -> bool {
    if (a.analysis.seconds - b.analysis.seconds).abs() > SAME_LENGTH {
        return false;
    }
    let (a, b) = (a.fingerprint(), b.fingerprint());
    (!a.name.is_empty() && a.name == b.name) || dot(&a.sound, &b.sound) > 0.98
}

/// How far apart two copies of a recording's lengths can be, seconds.
const SAME_LENGTH: f64 = 2.0;

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() as f64
}

/// `transitions` with one per recording, best first kept: copies are looked
/// for only among what is about as long, so a long list costs no more than
/// its length.
fn one_per_recording(transitions: Vec<Transition>, others: &[Track]) -> Vec<Transition> {
    let mut kept: Vec<Transition> = Vec::with_capacity(transitions.len());
    let mut by_length: std::collections::HashMap<i64, Vec<usize>> = std::collections::HashMap::new();
    for transition in transitions {
        let track = &others[transition.to];
        let second = track.analysis.seconds.round() as i64;
        let reach = SAME_LENGTH.ceil() as i64 + 1;
        let copy = (second - reach..=second + reach)
            .filter_map(|s| by_length.get(&s))
            .flatten()
            .any(|&k| same_recording(&others[kept[k].to], track));
        if !copy {
            by_length.entry(second).or_default().push(kept.len());
            kept.push(transition);
        }
    }
    kept
}

/// The closest any phrase late in `playing` comes to one early in `next`,
/// if `next` has a phrase to come in on.
fn best_sound(playing: &Track, next: &Track) -> Option<f64> {
    let ins = entries(next);
    half(playing, false)
        .iter()
        .flat_map(|&out| ins.iter().map(move |&into| cosine(&playing.segments[out].embedding, &next.segments[into].embedding)))
        .max_by(f64::total_cmp)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transition {
    /// Index of the incoming track in the list it was chosen from.
    pub to: usize,
    /// Segment of the playing track to mix out of.
    pub out: usize,
    /// Segment of the incoming track to mix into.
    pub into: usize,
    pub score: f64,
    pub sound: f64,
    /// Incoming tempo over the one it will play at, the outgoing track's or
    /// the set's, octaves folded: 1.02 is 2% faster.
    pub ratio: f64,
    pub key_steps: Option<u32>,
    /// Incoming loudness minus outgoing, LU.
    pub energy: Option<f64>,
}

/// The best transition into each of `others`, best first. The playing track
/// is mixed out of from its second half, the next one mixed into in its first.
pub fn from(playing: &Track, others: &[Track], options: &Options) -> Vec<Transition> {
    from_allowed(playing, others, options, |_, _| true)
}

/// As `from`, over only the `others` that `allowed` lets through: a set's
/// planner leaves out what it has played and what is too far from its tempo.
pub fn from_allowed(
    playing: &Track,
    others: &[Track],
    options: &Options,
    allowed: impl Fn(usize, &Track) -> bool + Sync,
) -> Vec<Transition> {
    let Some(bpm) = options.tempo.or(playing.bpm()).filter(|b| b.is_finite() && *b > 0.0) else {
        return Vec::new();
    };
    let outs = half(playing, false);

    // Each track's best pair of phrases first, by how alike they sound and
    // how evenly the energy carries over.
    let candidate = |(to, next): (usize, &Track)| -> Option<(Transition, f64, f64)> {
        if next.path == playing.path || !allowed(to, next) || same_recording(playing, next) {
            return None;
        }
        {
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

            let ins = entries(next);
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
        }
    };
    // Over every core for a large library, in order either way.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut best: Vec<(Transition, f64, f64)> = if others.len() < 2000 || threads < 2 {
        others.iter().enumerate().filter_map(candidate).collect()
    } else {
        let chunk = others.len().div_ceil(threads);
        std::thread::scope(|scope| {
            let handles: Vec<_> = others
                .chunks(chunk)
                .enumerate()
                .map(|(c, part)| {
                    let candidate = &candidate;
                    scope.spawn(move || part.iter().enumerate().filter_map(|(i, t)| candidate((c * chunk + i, t))).collect::<Vec<_>>())
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("a scoring thread panicked")).collect()
        })
    };

    // Then sound scored against the library's best pairs: within a genre
    // every pair is 0.93–0.96, so the raw cosine would hardly count.
    for (transition, fixed, energy_score) in &mut best {
        transition.score = SOUND * options.sound.score(transition.sound) + *fixed + ENERGY * *energy_score;
    }
    let mut best: Vec<Transition> = best.into_iter().map(|b| b.0).collect();
    best.sort_by(|a, b| b.score.total_cmp(&a.score));
    // One way into each recording: the best of its copies.
    one_per_recording(best, others)
}

/// The segments a track can be mixed into: those in its first half that
/// start on a downbeat, so the bars of the two tracks line up. A lead-in
/// from the first beat that is not a bar's first is left out.
fn entries(track: &Track) -> Vec<usize> {
    let downbeats = &track.analysis.downbeats;
    let on_bar = |i: &usize| {
        let start = track.segments[*i].start;
        downbeats.iter().any(|d| (d - start).abs() < 1e-3)
    };
    let first = half(track, true);
    let aligned: Vec<usize> = first.iter().copied().filter(on_bar).collect();
    // A track without downbeats to go by still gets its first half.
    if aligned.is_empty() && downbeats.is_empty() {
        first
    } else {
        aligned
    }
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
/// after an 87 one is a 1.0, not a 2.0. A ratio that is no tempo, of or to
/// zero, is infinitely far: no octave brings it near.
pub fn fold(mut ratio: f64) -> f64 {
    if !ratio.is_finite() || ratio <= 0.0 {
        return f64::INFINITY;
    }
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
pub(crate) mod tests {
    use super::*;
    use crate::segments::Segment;
    use crate::{Analysis, Grid};

    /// A 120 BPM track of 8-bar phrases (16s each), whose phrase `i` sounds
    /// like direction `sounds[i]` plus a little of a shared one.
    pub(crate) fn track(name: &str, sounds: &[usize], lead_in_off_bar: bool) -> Track {
        let phrase = 16.0;
        let embedding = |d: usize| {
            let mut v = vec![0.2f32; 512];
            v[d] = 1.0;
            v
        };
        let mut segments: Vec<Segment> = sounds
            .iter()
            .enumerate()
            .map(|(i, &d)| Segment {
                start: 8.0 + i as f64 * phrase,
                end: 8.0 + (i + 1) as f64 * phrase,
                loudness: Some(-8.0),
                brightness: None,
                onset_rate: None,
                key: None,
                embedding: embedding(d),
            })
            .collect();
        // A tail of its own length after the phrases, so that no two test
        // tracks pass for copies of one recording.
        let tail = name.bytes().map(f64::from).sum::<f64>() % 10.0 * 3.0;
        let seconds = 8.0 + sounds.len() as f64 * phrase + tail;
        let downbeats: Vec<f64> = (0..).map(|b| 8.0 + b as f64 * 2.0).take_while(|&d| d < seconds).collect();
        if lead_in_off_bar {
            // A lead-in from a beat that is not a bar's first, sounding like
            // anything at all.
            segments.insert(0, Segment { start: 1.5, end: 8.0, embedding: embedding(0), ..segments[0].clone() });
        }
        Track {
            path: name.into(),
            size: 0,
            modified: 0,
            version: crate::library::VERSION,
            bpm_range: crate::tempo::DJ_BPM,
            origin: None,
            fingerprint: Default::default(),
            analysis: Analysis {
                seconds,
                grid: Some(Grid { bpm: 120.0, first_beat: 0.0, rms_ms: 0.0, on_grid: 1.0, steady: true }),
                key: None,
                loudness: Some(-8.0),
                beats: (0..).map(|b| b as f64 * 0.5).take_while(|&t| t < seconds).collect(),
                downbeats,
                downbeat_confidence: 1.0,
                phrases: Vec::new(),
                sections: Vec::new(),
            },
            segments,
        }
    }

    #[test]
    fn a_pair_scores_the_same_whatever_else_is_allowed() {
        let playing = track("a", &[1, 1, 2, 2], false);
        let library: Vec<Track> = (0..6).map(|i| track(&format!("t{i}"), &[2 + i % 3, 3, 4, 5], false)).collect();
        let options = Options {
            sound: SoundScale::of(&library),
            ..Default::default()
        };
        let alone = from_allowed(&playing, &library, &options, |i, _| i == 0);
        let among = from_allowed(&playing, &library, &options, |_, _| true);
        let score = |ts: &[Transition]| ts.iter().find(|t| t.to == 0).unwrap().score;
        assert_eq!(score(&alone), score(&among));
        // And alone it is not a middling 0.5 by default: it sounds like the
        // playing track's end, more than most pairs do.
        assert!(score(&alone) > 0.8, "{}", score(&alone));
    }

    #[test]
    fn nothing_is_mixed_into_a_lead_in_that_starts_off_the_bar() {
        let playing = track("a", &[1, 1, 7, 7], false);
        // The off-bar lead-in sounds just like the playing track's end; the
        // phrases do not.
        let next = track("b", &[3, 4, 5, 6], true);
        let mut lead = next.clone();
        lead.segments[0].embedding = playing.segments[3].embedding.clone();
        let best = &from(&playing, std::slice::from_ref(&lead), &Options::default())[0];
        assert!(lead.analysis.downbeats.contains(&lead.segments[best.into].start), "into {}", best.into);
        assert_ne!(best.into, 0);
    }

    #[test]
    fn copies_of_a_recording_are_one_recording() {
        let a = track("03 - HANK J", &[1, 2, 3], false);
        let mut renamed = a.clone();
        renamed.path = "01 - Hank J..flac".into();
        assert!(same_recording(&a, &renamed), "by name, numbers and dots aside");
        let mut ripped = a.clone();
        ripped.path = "hank j official video [x8Fz]".into();
        assert!(same_recording(&a, &ripped), "by sound, under another name");
        ripped.analysis.seconds += 10.0;
        assert!(!same_recording(&a, &ripped), "an extended mix is another recording");
        // Nothing is mixed into a copy of what is playing.
        assert!(from(&a, std::slice::from_ref(&renamed), &Options::default()).is_empty());
        // And of two copies to go into, the next list holds one.
        let playing = track("c", &[1, 2, 3], false);
        assert_eq!(from(&playing, &[a.clone(), renamed.clone()], &Options::default()).len(), 1);
        let other = track("b", &[4, 5, 6], false);
        assert!(!same_recording(&a, &other));
    }

    #[test]
    fn a_tempo_of_zero_finds_nothing_rather_than_hanging() {
        let playing = track("a", &[1, 2], false);
        let others = vec![track("b", &[1, 2], false)];
        let options = Options { tempo: Some(0.0), ..Default::default() };
        assert!(from(&playing, &others, &options).is_empty());
    }

    #[test]
    fn octaves_fold_away() {
        assert!((fold(174.0 / 87.0) - 1.0).abs() < 1e-9);
        assert!((fold(160.0 / 155.0) - 160.0 / 155.0).abs() < 1e-9);
        assert!((fold(80.0 / 155.0) - 160.0 / 155.0).abs() < 1e-9);
        // No tempo is no answer, and no hang.
        for nonsense in [0.0, -1.0, f64::INFINITY, f64::NAN, 120.0 / 0.0] {
            assert_eq!(fold(nonsense), f64::INFINITY, "{nonsense}");
        }
    }
}
