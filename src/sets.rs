//! The sets kept: one JSON file a set under `$XDG_DATA_HOME/stelly-mix/sets`,
//! as the library keeps its tracks.
//!
//! A `set::Plan` names its tracks by where they sit in the library it was
//! planned against, which is whatever order the entries were read in that
//! time. Kept as they are, a set would point at different music the next
//! morning. A `Set` names them by their origin key instead, `stelly:<id>`,
//! which is the track itself, and resolves back to a plan against whatever
//! library is loaded now.
//!
//! A track can leave the library between saving a set and opening it. That
//! is a hole in the set, not a reason to lose the rest of it, so `resolve`
//! reports what is missing and leaves the caller to decide.

use crate::library::Track;
use crate::set::{Options, Plan};
use crate::transitions::{SoundScale, Transition};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Bump when a kept set would read differently.
pub const VERSION: u32 = 1;

/// A set as it is kept: its tracks by origin, and how to get from each into
/// the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Set {
    pub id: String,
    pub name: String,
    pub version: u32,
    /// Unix seconds.
    pub created: u64,
    pub modified: u64,
    /// Origin keys, `source:id`, in playing order.
    pub tracks: Vec<String>,
    /// `steps[k]` takes `tracks[k]` into `tracks[k + 1]`, so there is one
    /// fewer of them than there are tracks.
    pub steps: Vec<Step>,
    /// Seconds up to the last transition, as the plan reckoned them.
    pub seconds: f64,
    pub mean_score: f64,
    /// What it was planned on, so it can be planned again on the same terms.
    pub planned: Planned,
}

/// A transition, without the `to` that a plan carries: in a kept set the
/// track mixed into is simply the next one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub out: usize,
    pub into: usize,
    pub score: f64,
    pub sound: f64,
    pub ratio: f64,
    pub key_steps: Option<u32>,
    pub energy: Option<f64>,
}

impl Step {
    fn of(transition: &Transition) -> Self {
        let Transition { out, into, score, sound, ratio, key_steps, energy, .. } = *transition;
        Self { out, into, score, sound, ratio, key_steps, energy }
    }

    /// Back as the planner and the renderer take it, the next track at `to`.
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

/// The terms a set was planned on. Not the whole of `set::Options`: `like`
/// is hundreds of scores against a library that has since changed, and is
/// kept as the words it came from instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Planned {
    pub minutes: f64,
    /// Largest tempo change from the set's, as a fraction.
    pub max_stretch: f64,
    pub beam: usize,
    pub min_transition: f64,
    /// What the set was asked to sound like, if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub like: Option<String>,
}

impl Default for Planned {
    fn default() -> Self {
        let defaults = Options::default();
        Self {
            minutes: defaults.seconds / 60.0,
            max_stretch: defaults.max_stretch,
            beam: defaults.beam,
            min_transition: defaults.min_transition,
            like: None,
        }
    }
}

impl Planned {
    /// Back as the planner takes them. `like` is left for the caller to
    /// score, which wants the text tower.
    pub fn options(&self) -> Options {
        Options {
            seconds: self.minutes * 60.0,
            max_stretch: self.max_stretch,
            beam: self.beam,
            min_transition: self.min_transition,
            ..Options::default()
        }
    }
}

/// What a kept set came to against the library as it is now.
pub struct Resolved {
    pub plan: Plan,
    /// The origin keys the library no longer holds, in the order they were
    /// kept. Empty is a set that came back whole.
    pub missing: Vec<String>,
}

impl Set {
    /// A set of `name` from `plan` against `library`, the tracks it names
    /// taken by their origin. A track of no origin — a local file — cannot
    /// be named stably, so it is refused rather than kept as a path that
    /// will not mean the same thing elsewhere.
    pub fn of(name: &str, plan: &Plan, library: &[Track], planned: Planned) -> Result<Self> {
        let tracks = plan
            .tracks
            .iter()
            .map(|&index| {
                let track = library.get(index).with_context(|| format!("the plan names track {index}, which the library has not"))?;
                let origin = track.origin.as_ref().with_context(|| format!("{} came from no source, so a set cannot keep it", track.name()))?;
                Ok(origin.key())
            })
            .collect::<Result<Vec<String>>>()?;
        let now = now();
        Ok(Self {
            id: id_for(name, now),
            name: name.trim().to_string(),
            version: VERSION,
            created: now,
            modified: now,
            tracks,
            steps: plan.transitions.iter().map(Step::of).collect(),
            seconds: plan.seconds,
            mean_score: plan.mean_score(),
            planned,
        })
    }

    /// The set against `library` as it is now: a plan the renderer and the
    /// views take, and whatever has gone missing since.
    ///
    /// A missing track takes its step with it, and the set closes up around
    /// it: what is left still plays, with one fewer way through it.
    pub fn resolve(&self, library: &[Track]) -> Resolved {
        let found: Vec<Option<usize>> = self.tracks.iter().map(|key| index_of(library, key)).collect();
        let missing = self.tracks.iter().zip(&found).filter(|(_, at)| at.is_none()).map(|(key, _)| key.clone()).collect();
        let mut tracks = Vec::new();
        let mut transitions = Vec::new();
        for (position, at) in found.iter().enumerate() {
            let Some(&at) = at.as_ref() else { continue };
            // The step out of the track before, kept only when both ends of
            // it are still here: a step across a hole is not the mix that
            // was planned.
            if let Some(previous) = position.checked_sub(1) {
                if !tracks.is_empty() && found[previous].is_some() {
                    if let Some(step) = self.steps.get(previous) {
                        transitions.push(step.transition(at));
                    }
                }
            }
            tracks.push(at);
        }
        // A chain the renderer takes has one transition between each pair.
        transitions.truncate(tracks.len().saturating_sub(1));
        let seconds = if transitions.len() == self.steps.len() { self.seconds } else { 0.0 };
        let score: f64 = transitions.iter().map(|t| t.score).sum();
        Resolved { plan: Plan { tracks, transitions, seconds, score }, missing }
    }

    /// Re-cut the ways through the set after its tracks have moved.
    ///
    /// Each pair left next to each other takes the best transition there is
    /// between those two, ranked against the set's own tempo as the planner
    /// ranks them — a reordered set is not the planner's choice any more, but
    /// every mix in it is still the best one available for that pair.
    ///
    /// Refused rather than fudged when a pair has no way between them inside
    /// the tempo range: a set with a gap in it is not something to hand to
    /// the decks without being told.
    pub fn recut(&mut self, library: &[Track], max_stretch: f64, sound: SoundScale) -> Result<()> {
        let at: Vec<usize> = self
            .tracks
            .iter()
            .map(|key| index_of(library, key).with_context(|| format!("{key} is not in the library")))
            .collect::<Result<_>>()?;
        let Some(&first) = at.first() else {
            self.steps.clear();
            self.seconds = 0.0;
            self.mean_score = 0.0;
            self.modified = now();
            return Ok(());
        };
        let master = library[first].bpm().context("the first track has no tempo to hold the set at")?;
        let ranking = crate::transitions::Options { max_stretch, tempo: Some(master), sound };

        let mut steps = Vec::new();
        for pair in at.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let best = crate::transitions::from_allowed(&library[from], library, &ranking, |i, _| i == to)
                .into_iter()
                .next()
                .with_context(|| {
                    format!(
                        "there is no way from {} into {} inside ±{:.0}%",
                        library[from].name(),
                        library[to].name(),
                        max_stretch * 100.0
                    )
                })?;
            steps.push(Step::of(&best));
        }

        // The same arithmetic the planner uses: each track plays from where
        // it came in to where it goes out, at the set's tempo.
        let rate = |track: &Track| track.bpm().map_or(1.0, |bpm| crate::transitions::fold(bpm / master));
        let mut seconds = 0.0;
        for (k, &index) in at.iter().enumerate() {
            let track = &library[index];
            let Some(out) = steps.get(k) else { break };
            let entered = k.checked_sub(1).and_then(|previous| steps.get(previous)).map_or(0.0, |step| track.segments[step.into].start);
            seconds += (track.segments[out.out].start - entered) * rate(track);
        }

        self.mean_score = steps.iter().map(|step| step.score).sum::<f64>() / steps.len().max(1) as f64;
        self.steps = steps;
        self.seconds = seconds;
        self.modified = now();
        Ok(())
    }

    /// What else could sit at `at`: tracks that mix well both out of the one
    /// before and into the one after, best first, as origin keys with the
    /// mean of the two mixes they would make.
    ///
    /// Ranked from the track before first, then only that shortlist is tried
    /// into the one after: scoring every track in the library against both
    /// neighbours is two passes over thousands of tracks for a menu of six.
    pub fn alternatives(&self, at: usize, library: &[Track], max_stretch: f64, sound: SoundScale, limit: usize) -> Vec<(String, f64)> {
        const SHORTLIST: usize = 80;
        let Some(&first) = self.tracks.first().and_then(|key| index_of(library, key)).as_ref() else {
            return Vec::new();
        };
        let Some(master) = library[first].bpm() else { return Vec::new() };
        let ranking = crate::transitions::Options { max_stretch, tempo: Some(master), sound };
        let resolved: Vec<Option<usize>> = self.tracks.iter().map(|key| index_of(library, key)).collect();
        // Everything the set already holds, the slot's own track included:
        // offering to swap a track for itself is a menu item that does
        // nothing.
        let held: Vec<usize> = resolved.iter().flatten().copied().collect();
        let free = |i: usize| !held.contains(&i);

        let before = at.checked_sub(1).and_then(|k| resolved.get(k).copied().flatten());
        let after = resolved.get(at + 1).copied().flatten();

        // Out of the track before, or — for the first slot — into the one
        // after, which is the only mix that slot has.
        let shortlist: Vec<(usize, f64)> = match (before, after) {
            (Some(before), _) => crate::transitions::from_allowed(&library[before], library, &ranking, |i, _| free(i))
                .into_iter()
                .take(SHORTLIST)
                .map(|step| (step.to, step.score))
                .collect(),
            (None, Some(after)) => (0..library.len())
                .filter(|&i| free(i))
                .filter_map(|i| {
                    let step = crate::transitions::from_allowed(&library[i], library, &ranking, |j, _| j == after).into_iter().next()?;
                    Some((i, step.score))
                })
                .collect(),
            (None, None) => return Vec::new(),
        };

        let mut scored: Vec<(String, f64)> = shortlist
            .into_iter()
            .filter_map(|(candidate, incoming)| {
                let score = match (before, after) {
                    // Both neighbours: the slot is only as good as its worse side.
                    (Some(_), Some(after)) => {
                        let out = crate::transitions::from_allowed(&library[candidate], library, &ranking, |j, _| j == after)
                            .into_iter()
                            .next()?
                            .score;
                        (incoming + out) / 2.0
                    }
                    _ => incoming,
                };
                let key = library[candidate].origin.as_ref()?.key();
                Some((key, score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(limit);
        scored
    }

    /// Keep the set up to and including `at`, and plan the rest again from
    /// there — the half you liked held, the half you did not chosen afresh.
    ///
    /// What is kept is handed to the planner as tracks to avoid, so the new
    /// tail cannot play something the head already did.
    pub fn replan_from(&mut self, at: usize, library: &[Track], mut options: Options) -> Result<()> {
        let resolved: Vec<usize> = self
            .tracks
            .iter()
            .map(|key| index_of(library, key).with_context(|| format!("{key} is not in the library")))
            .collect::<Result<_>>()?;
        let pivot = *resolved.get(at).context("there is no track there to plan from")?;

        // How long the kept head already runs, so the tail is planned for
        // what is left of the set's length rather than its whole length.
        let kept_seconds: f64 = {
            let mut set = self.clone();
            set.tracks.truncate(at + 1);
            set.steps.truncate(at);
            set.recut(library, options.max_stretch, crate::transitions::SoundScale::of(library)).ok();
            set.seconds
        };
        options.seconds = (options.seconds - kept_seconds).max(0.0);
        options.avoid = resolved[..at].to_vec();

        let tail = crate::set::plan(library, pivot, &options).context("there is nowhere to go from that track")?;
        let keys: Vec<String> = tail
            .tracks
            .iter()
            .map(|&i| library[i].origin.as_ref().map(|origin| origin.key()).context("a planned track came from no source"))
            .collect::<Result<_>>()?;

        self.tracks.truncate(at);
        self.tracks.extend(keys);
        self.recut(library, options.max_stretch, crate::transitions::SoundScale::of(library))
    }

    /// Move the track at `from` to `to`, and re-cut around it.
    pub fn reorder(&mut self, from: usize, to: usize, library: &[Track], max_stretch: f64, sound: SoundScale) -> Result<()> {
        if from >= self.tracks.len() || to >= self.tracks.len() || from == to {
            return Ok(());
        }
        let moved = self.tracks.remove(from);
        self.tracks.insert(to, moved);
        self.recut(library, max_stretch, sound)
    }

    /// Take the track at `at` out, closing the set up around it.
    pub fn remove(&mut self, at: usize, library: &[Track], max_stretch: f64, sound: SoundScale) -> Result<()> {
        if at >= self.tracks.len() {
            return Ok(());
        }
        self.tracks.remove(at);
        self.recut(library, max_stretch, sound)
    }

    /// Put `key` in the slot at `at`, and re-cut the two mixes it touches.
    pub fn replace(&mut self, at: usize, key: &str, library: &[Track], max_stretch: f64, sound: SoundScale) -> Result<()> {
        if at >= self.tracks.len() {
            return Ok(());
        }
        self.tracks[at] = key.to_string();
        self.recut(library, max_stretch, sound)
    }

    /// Rename it, keeping its id: the id names the file, and a set that is
    /// renamed is the same set.
    pub fn rename(&mut self, name: &str) {
        self.name = name.trim().to_string();
        self.modified = now();
    }

    /// A copy under a new name and a new id, as "duplicate" makes.
    pub fn duplicate(&self, name: &str) -> Self {
        let now = now();
        Self { id: id_for(name, now), name: name.trim().to_string(), created: now, modified: now, ..self.clone() }
    }

    /// How long it runs, minutes and seconds, as every other clock in the
    /// window reads: hours and minutes made a 13-minute set look like 13
    /// seconds.
    pub fn clock(&self) -> String {
        let seconds = self.seconds.max(0.0).round() as u64;
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Where the sets are kept.
pub struct Sets {
    dir: PathBuf,
}

impl Sets {
    pub fn open(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn default_dir() -> PathBuf {
        crate::models::data_home().join("stelly-mix/sets")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn get(&self, id: &str) -> Option<Set> {
        serde_json::from_slice(&std::fs::read(self.file(id)).ok()?).ok()
    }

    /// Every set kept, newest first. One that will not read is passed over
    /// rather than taken for the lot.
    pub fn all(&self) -> Result<Vec<Set>> {
        let mut sets: Vec<Set> = std::fs::read_dir(&self.dir)?
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| path.extension().is_some_and(|e| e == "json"))
            .filter_map(|path| serde_json::from_slice::<Set>(&std::fs::read(path).ok()?).ok())
            .filter(|set| set.version == VERSION)
            .collect();
        sets.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
        Ok(sets)
    }

    /// Written aside and moved into place, so a set is never half a file.
    pub fn put(&self, set: &Set) -> Result<()> {
        if set.id.is_empty() {
            bail!("a set wants an id to be kept under");
        }
        let file = self.file(&set.id);
        let partial = file.with_extension(format!("{}.part", std::process::id()));
        std::fs::write(&partial, serde_json::to_vec_pretty(set)?).with_context(|| format!("writing {}", partial.display()))?;
        std::fs::rename(&partial, &file).with_context(|| format!("moving {} into place", file.display()))?;
        Ok(())
    }

    /// Forget it. Whether there was one.
    pub fn remove(&self, id: &str) -> Result<bool> {
        match std::fs::remove_file(self.file(id)) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| format!("forgetting set {id}")),
        }
    }
}

/// Where `key` sits in `library`, by origin.
fn index_of(library: &[Track], key: &str) -> Option<usize> {
    library.iter().position(|track| track.origin.as_ref().is_some_and(|origin| origin.key() == key))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

/// A file name for a set: the name as far as it is safe, and a hash of the
/// name and the moment, so two sets called the same thing are two files.
fn id_for(name: &str, at: u64) -> String {
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<&str>>()
        .join("-");
    let slug: String = slug.chars().take(48).collect();
    let digest = Sha256::digest(format!("{name}/{at}").as_bytes());
    let suffix: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    if slug.is_empty() {
        format!("set-{suffix}")
    } else {
        format!("{slug}-{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Item, Origin};

    fn track(id: &str) -> Track {
        let mut track = crate::transitions::tests::track(id, &[0, 1, 2, 3], false);
        track.origin = Some(Origin {
            source: "stelly".into(),
            item: Item { id: id.into(), artist: "An Artist".into(), title: format!("Track {id}"), seconds: Some(240.0) },
        });
        track
    }

    fn plan(tracks: Vec<usize>) -> Plan {
        let transitions = tracks
            .windows(2)
            .map(|pair| Transition { to: pair[1], out: 1, into: 0, score: 0.5, sound: 0.9, ratio: 1.0, key_steps: Some(0), energy: Some(0.0) })
            .collect();
        Plan { tracks, transitions, seconds: 600.0, score: 1.0 }
    }

    #[test]
    fn a_set_keeps_its_tracks_by_origin_not_by_where_they_sat() {
        let library = vec![track("1"), track("2"), track("3")];
        let set = Set::of("Friday", &plan(vec![0, 2]), &library, Planned::default()).unwrap();
        assert_eq!(set.tracks, vec!["stelly:1", "stelly:3"]);

        // The same tracks, read in another order, as another morning gives them.
        let shuffled = vec![track("3"), track("9"), track("1")];
        let resolved = set.resolve(&shuffled);
        assert!(resolved.missing.is_empty());
        assert_eq!(resolved.plan.tracks, vec![2, 0], "the set follows its tracks, not their old places");
        assert_eq!(resolved.plan.transitions.len(), 1);
        assert_eq!(resolved.plan.transitions[0].to, 0);
    }

    #[test]
    fn a_track_that_has_left_the_library_is_a_hole_not_a_loss() {
        let library = vec![track("1"), track("2"), track("3")];
        let set = Set::of("Friday", &plan(vec![0, 1, 2]), &library, Planned::default()).unwrap();

        let resolved = set.resolve(&[track("1"), track("3")]);
        assert_eq!(resolved.missing, vec!["stelly:2"]);
        assert_eq!(resolved.plan.tracks.len(), 2, "the rest of the set still plays");
        // The step across the hole was not the mix that was planned.
        assert!(resolved.plan.transitions.is_empty());
    }

    #[test]
    fn a_track_from_no_source_cannot_be_kept() {
        let mut local = track("1");
        local.origin = None;
        let error = Set::of("Friday", &plan(vec![0]), &[local], Planned::default()).unwrap_err();
        assert!(format!("{error:#}").contains("no source"), "{error:#}");
    }

    #[test]
    fn ids_are_a_readable_slug_and_differ_for_one_name() {
        assert!(id_for("Friday night", 0).starts_with("friday-night-"));
        assert!(id_for("  Déjà vu / 2  ", 0).starts_with("d-j-vu-2-"));
        assert!(id_for("???", 0).starts_with("set-"));
        assert_ne!(id_for("Friday", 1), id_for("Friday", 2));
    }

    /// A library whose tracks can actually mix: the shared test track is
    /// built to, and they all sit at one tempo.
    fn mixable() -> Vec<Track> {
        (1..=5).map(|n| track(&n.to_string())).collect()
    }

    /// A set over `mixable()`, cut by the planner's own rules.
    fn cut(order: &[usize], library: &[Track]) -> Set {
        let mut set = Set::of("Friday", &plan(order.to_vec()), library, Planned::default()).unwrap();
        set.recut(library, 0.06, SoundScale::of(library)).expect("the test tracks mix");
        set
    }

    #[test]
    fn re_cutting_finds_a_way_for_every_pair_left_next_to_each_other() {
        let library = mixable();
        let set = cut(&[0, 1, 2], &library);
        assert_eq!(set.steps.len(), 2, "one way between each pair");
        assert!(set.seconds > 0.0, "a set that plays has a length");
        assert!(set.mean_score > 0.0);
    }

    #[test]
    fn reordering_re_cuts_around_the_track_that_moved() {
        let library = mixable();
        let mut set = cut(&[0, 1, 2], &library);
        let before = set.steps.clone();
        set.reorder(2, 0, &library, 0.06, SoundScale::of(&library)).unwrap();
        assert_eq!(set.tracks, vec!["stelly:3", "stelly:1", "stelly:2"]);
        assert_eq!(set.steps.len(), 2);
        assert_ne!(set.steps, before, "the ways are chosen again for the new neighbours");
    }

    #[test]
    fn removing_a_track_closes_the_set_up_around_it() {
        let library = mixable();
        let mut set = cut(&[0, 1, 2], &library);
        set.remove(1, &library, 0.06, SoundScale::of(&library)).unwrap();
        assert_eq!(set.tracks, vec!["stelly:1", "stelly:3"]);
        assert_eq!(set.steps.len(), 1, "the two that are left are mixed directly");
    }

    #[test]
    fn swapping_a_slot_takes_the_track_put_there() {
        let library = mixable();
        let mut set = cut(&[0, 1, 2], &library);
        set.replace(1, "stelly:4", &library, 0.06, SoundScale::of(&library)).unwrap();
        assert_eq!(set.tracks, vec!["stelly:1", "stelly:4", "stelly:3"]);
        assert_eq!(set.steps.len(), 2);
    }

    #[test]
    fn alternatives_leave_out_what_the_set_already_holds() {
        let library = mixable();
        let set = cut(&[0, 1, 2], &library);
        let offered = set.alternatives(1, &library, 0.06, SoundScale::of(&library), 5);
        assert!(!offered.is_empty(), "the library has other tracks to offer");
        for (key, _) in &offered {
            assert_ne!(key, "stelly:1", "the slot's neighbours are not alternatives to it");
            assert_ne!(key, "stelly:3");
            assert_ne!(key, "stelly:2", "nor is the track already in the slot");
        }
        // Best first.
        assert!(offered.windows(2).all(|pair| pair[0].1 >= pair[1].1));
    }

    #[test]
    fn re_planning_holds_the_head_and_never_repeats_it() {
        let library = mixable();
        let mut set = cut(&[0, 1, 2], &library);
        let head = set.tracks[..2].to_vec();

        let mut options = Planned::default().options();
        options.seconds = 60.0;
        options.min_play = 0.0;
        options.min_transition = 0.0;
        set.replan_from(1, &library, options).unwrap();

        assert_eq!(&set.tracks[..2], &head[..], "what was kept is still there, in order");
        // Nothing in the new tail repeats the head.
        let mut seen = std::collections::HashSet::new();
        for key in &set.tracks {
            assert!(seen.insert(key.clone()), "{key} plays twice");
        }
        assert_eq!(set.steps.len(), set.tracks.len() - 1);
    }

    #[test]
    fn an_edit_that_leaves_no_way_through_is_refused() {
        let mut library = mixable();
        // A track at a tempo nothing here can reach inside ±6%.
        let mut far = track("9");
        far.analysis.grid = far.analysis.grid.map(|mut grid| {
            grid.bpm = 300.0;
            grid
        });
        library.push(far);
        let mut set = cut(&[0, 1], &library);
        let kept = set.tracks.clone();
        let error = set.replace(1, "stelly:9", &library, 0.06, SoundScale::of(&library)).unwrap_err();
        assert!(format!("{error:#}").contains("no way from"), "{error:#}");
        // The set is left naming what was asked for; the caller re-reads it.
        assert_ne!(set.tracks, kept);
    }

    #[test]
    fn sets_are_kept_and_read_back_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("stelly-mix-sets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sets = Sets::open(dir.clone()).unwrap();
        assert!(sets.all().unwrap().is_empty());

        let library = vec![track("1"), track("2")];
        let mut set = Set::of("Friday", &plan(vec![0, 1]), &library, Planned::default()).unwrap();
        sets.put(&set).unwrap();
        assert_eq!(sets.get(&set.id).as_ref(), Some(&set));
        assert_eq!(sets.all().unwrap().len(), 1);

        set.rename("Saturday");
        sets.put(&set).unwrap();
        assert_eq!(sets.all().unwrap().len(), 1, "a rename keeps the id, so it is still one set");
        assert_eq!(sets.get(&set.id).unwrap().name, "Saturday");

        let copy = set.duplicate("Saturday, again");
        sets.put(&copy).unwrap();
        assert_eq!(sets.all().unwrap().len(), 2);
        assert_eq!(copy.tracks, set.tracks);

        assert!(sets.remove(&set.id).unwrap());
        assert!(!sets.remove(&set.id).unwrap(), "forgetting it twice is not an error");
        assert_eq!(sets.all().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
