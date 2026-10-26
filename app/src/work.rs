//! What the app asks of stelly-mix, each a plain function that can take
//! seconds, for the window to run on tokio's blocking pool: loading the
//! library, a track's waveform, where to go next, a transition or a set
//! rendered to a file, and the library searched by description.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use stelly_mix::library::{Library, Track};
use stelly_mix::render::{self, Settings};
use stelly_mix::set::{self, Plan};
use stelly_mix::source::{Source, Sources};
use stelly_mix::text::TextEncoder;
use stelly_mix::transitions::{self, SoundScale, Transition};

/// Points across a waveform: enough for a wide window, few enough to draw.
pub const PEAKS: usize = 1600;
/// Seconds of each track either side of a rendered transition.
pub const LEAD: f64 = 20.0;

static SOURCES: OnceLock<Sources> = OnceLock::new();

/// The sources besides local files this build was started with.
pub fn set_sources(sources: Vec<Box<dyn Source>>) {
    let _ = SOURCES.set(Sources::new(sources));
}

pub fn sources() -> &'static Sources {
    SOURCES.get_or_init(|| Sources::new(Vec::new()))
}

/// Run `work` on tokio's blocking pool, so the window keeps drawing.
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.context("the work panicked")?
}

/// The library as it is on disk, and the scale its transitions are scored on.
pub struct Loaded {
    pub tracks: Vec<Track>,
    pub scale: SoundScale,
}

impl Loaded {
    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.tracks.iter().position(|t| t.path == path)
    }
}

/// Stelly's server, when one is set: its mix library is mirrored at start.
/// The server this window talks to. Replaceable, so settings can point it
/// somewhere else without a restart; leaked rather than locked, because
/// every caller holds it for the length of a blocking call.
static SERVER: Mutex<Option<&'static stelly_mix::stelly::Stelly>> = Mutex::new(None);
static SERVER_READ: OnceLock<()> = OnceLock::new();

pub fn stelly() -> Option<&'static stelly_mix::stelly::Stelly> {
    SERVER_READ.get_or_init(|| {
        if let Some(found) = stelly_mix::stelly::Stelly::from_env() {
            *SERVER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::leak(Box::new(found)));
        }
    });
    *SERVER.lock().unwrap_or_else(|e| e.into_inner())
}

/// Point the window at `base` with `token` from here on, and keep it for
/// the next run. The server is tried before it is kept: a token that does
/// not work should not replace one that does.
pub fn set_server(base: &str, token: &str) -> Result<stelly_mix::api::Status> {
    let server = stelly_mix::stelly::Stelly::new(base, token);
    let status = server.status()?;
    stelly_mix::stelly::save(base, token)?;
    SERVER_READ.get_or_init(|| {});
    *SERVER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::leak(Box::new(server)));
    Ok(status)
}

pub fn library() -> Result<Library> {
    Library::open(Library::default_dir())
}

pub fn load() -> Result<Loaded> {
    let tracks = library()?.all()?;
    let scale = SoundScale::of(&tracks);
    Ok(Loaded { tracks, scale })
}

/// A file's waveform: the loudest sample in each of `PEAKS` stretches, 0..1,
/// remembered for the next time it is shown.
pub fn peaks(path: &Path) -> Result<Arc<Vec<f32>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<f32>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(peaks) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return Ok(peaks.clone());
    }
    let pcm = stelly_mix::decode::decode_file(path)?;
    let peaks = Arc::new(peaks_of(&pcm.samples, PEAKS));
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), peaks.clone());
    Ok(peaks)
}

fn peaks_of(samples: &[f32], buckets: usize) -> Vec<f32> {
    if samples.is_empty() {
        return vec![0.0; buckets];
    }
    let peaks: Vec<f32> = (0..buckets)
        .map(|b| {
            let (from, to) = (b * samples.len() / buckets, ((b + 1) * samples.len() / buckets).max(b * samples.len() / buckets + 1));
            samples[from..to.min(samples.len())].iter().fold(0f32, |m, v| m.max(v.abs()))
        })
        .collect();
    let top = peaks.iter().cloned().fold(1e-6, f32::max);
    peaks.into_iter().map(|p| p / top).collect()
}

/// The drops of a track analysed before they were looked for, or by an
/// older build, from its audio at `file`, kept in its library entry for the
/// next time.
pub fn drops(track: &Track, file: &Path) -> Result<Vec<f64>> {
    let pcm = stelly_mix::decode::decode_file(file)?;
    let drops = stelly_mix::drops::of(&pcm, &track.analysis);
    // Into the entry as it is on disk now, not as it was loaded: something
    // else, listening for voices say, may have written it since.
    let library = library()?;
    let stored = match &track.origin {
        Some(origin) => library.by_origin(&origin.key()),
        None => library.get(&track.path),
    };
    if let Some(mut stored) = stored {
        stored.analysis.drops = Some(drops.clone());
        library.put(&stored)?;
    }
    Ok(drops.at)
}

/// A track's audio, fetched again from its source if its file has gone.
pub fn audio(track: &Track) -> Result<PathBuf> {
    sources().audio(track)
}

/// Where to go from `playing`, best first.
pub fn next(loaded: &Loaded, playing: usize, stretch: f64) -> Vec<Transition> {
    let options = transitions::Options {
        max_stretch: stretch,
        sound: loaded.scale,
        ..Default::default()
    };
    transitions::from(&loaded.tracks[playing], &loaded.tracks, &options)
}

/// Where renders go: `$XDG_CACHE_HOME/stelly-mix/renders`.
pub fn renders() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("stelly-mix/renders")
}

/// A render on disk, and where things happen in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub path: PathBuf,
    pub seconds: f64,
    pub title: String,
    pub marks: Vec<Mark>,
}

/// Something that happens in a clip: a track coming in, a transition, the
/// basses trading.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub at: f64,
    pub until: Option<f64>,
    pub label: String,
}

fn file_name(parts: &[&str]) -> String {
    let joined = parts.join("\u{0}");
    let hash = joined.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{hash:016x}.wav")
}

/// A set's render is its tracks and where each mix happens: the same tracks
/// re-cut are another render, and must not overwrite this one.
fn set_file_name(paths: &[&Path], transitions: &[Transition], settings: &Settings) -> String {
    let mut parts: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    parts.extend(transitions.iter().map(|t| format!("{} {}", t.out, t.into)));
    parts.push(format!("{} {}", settings.keylock, settings.bass_swap));
    file_name(&parts.iter().map(String::as_str).collect::<Vec<_>>())
}

/// One transition rendered, `LEAD` seconds either side.
pub fn render_transition(playing: &Track, next: &Track, transition: &Transition, settings: &Settings) -> Result<Clip> {
    let a = stelly_mix::decode::decode_file_stereo(&audio(playing)?)?;
    let b = stelly_mix::decode::decode_file_stereo(&audio(next)?)?;
    let rendered = render::render(playing, a, next, b, transition, LEAD, settings)?;
    let dir = renders();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(file_name(&[
        &playing.path.to_string_lossy(),
        &next.path.to_string_lossy(),
        &format!("{} {} {} {}", transition.out, transition.into, settings.keylock, settings.bass_swap),
    ]));
    render::write_wav(&path, &rendered.audio)?;
    let cue = &rendered.cues[0];
    let mut marks = Vec::new();
    if let Some((from, to)) = cue.transition {
        marks.push(Mark { at: from, until: Some(to), label: format!("into {}", next.name()) });
    }
    if let Some(swap) = cue.swap {
        marks.push(Mark { at: swap, until: None, label: "basses swap".into() });
    }
    Ok(Clip {
        path,
        seconds: rendered.audio.len() as f64 / rendered.audio.rate as f64,
        title: format!("{} → {}", playing.name(), next.name()),
        marks,
    })
}

/// A set planned from `first`.
pub fn plan(loaded: &Loaded, first: usize, options: &set::Options) -> Result<Plan> {
    set::plan(&loaded.tracks, first, options).context("the first track has no tempo")
}

/// A planned set rendered whole, every track coming in where it does.
pub fn render_set(loaded: &Loaded, plan: &Plan) -> Result<Clip> {
    let chain: Vec<&Track> = plan.tracks.iter().map(|&i| &loaded.tracks[i]).collect();
    let last_to = chain.last().map_or(0.0, |t| t.analysis.seconds);
    let rendered = render::render_chain(&chain, &plan.transitions, 0.0, last_to, &Settings::default(), |k| {
        stelly_mix::decode::decode_file_stereo(&audio(chain[k])?)
    })?;
    let dir = renders();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let paths: Vec<&Path> = chain.iter().map(|t| t.path.as_path()).collect();
    let path = dir.join(set_file_name(&paths, &plan.transitions, &Settings::default()));
    render::write_wav(&path, &rendered.audio)?;
    let marks = rendered
        .cues
        .iter()
        .zip(&chain)
        .map(|(cue, track)| Mark { at: cue.enters, until: cue.transition.map(|t| t.1), label: track.name() })
        .collect();
    Ok(Clip {
        path,
        seconds: rendered.audio.len() as f64 / rendered.audio.rate as f64,
        title: format!("a set of {} from {}", chain.len(), chain[0].name()),
        marks,
    })
}

/// What was liked on Stelly, newest first, as the pane lists it.
pub struct LikedNow {
    /// Library keys of those analysed, the newest `limit`.
    pub keys: Vec<String>,
    /// Liked but not analysed yet: the server has them queued.
    pub waiting: usize,
    /// Brought from the server just now: the library wants reading again.
    pub brought: usize,
}

/// What the family liked on Stelly, newest first. Those the server has
/// analysed since the last sync are brought in on the way.
pub fn liked(loaded: &Loaded, limit: usize) -> Result<LikedNow> {
    let server = stelly().context("no Stelly server is set: connect one in Settings")?;
    let liked = server.liked(0)?;
    let here: std::collections::HashSet<String> = loaded.tracks.iter().filter_map(|t| Some(t.origin.as_ref()?.key())).collect();
    let missing: Vec<i64> = liked.iter().filter(|l| l.analysed && !here.contains(&l.key())).map(|l| l.track_id).collect();
    let brought = if missing.is_empty() { 0 } else { server.bring(&library()?, &missing, &|_, _| {})? };
    let keys = liked
        .iter()
        .filter(|l| here.contains(&l.key()) || (l.analysed && brought > 0))
        .map(|l| l.key())
        .take(limit)
        .collect();
    let waiting = liked.iter().filter(|l| !l.analysed && !here.contains(&l.key())).count();
    Ok(LikedNow { keys, waiting, brought })
}

/// Library tracks whose name holds every word of `words`, best match first
/// by name length: a short name containing the words is more likely the one
/// meant than a long one that happens to.
pub fn search_library(loaded: &Loaded, words: &str, limit: usize) -> Vec<usize> {
    let wanted: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<usize> = (0..loaded.tracks.len())
        .filter(|&i| {
            let name = loaded.tracks[i].name().to_lowercase();
            wanted.iter().all(|word| name.contains(word.as_str()))
        })
        .collect();
    hits.sort_by_key(|&i| loaded.tracks[i].name().len());
    hits.truncate(limit);
    hits
}

/// The library ranked against a description, best first, as origin keys
/// with their rank 0..1. The text tower is loaded on the first call.
pub fn sounds_like(loaded: &Loaded, words: &str, limit: usize) -> Result<Vec<(String, f64)>> {
    if words.trim().is_empty() {
        return Ok(Vec::new());
    }
    let ranks = like(loaded, words)?;
    let mut scored: Vec<(String, f64)> = loaded
        .tracks
        .iter()
        .zip(&ranks)
        .filter_map(|(track, &rank)| Some((track.origin.as_ref()?.key(), rank)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(limit);
    Ok(scored)
}

// ----------------------------------------------------------------- covers

/// Cover art by the server's track id, remembered for as long as the window
/// is open: the catalogue's artwork does not change under us, and asking
/// again for a cover already on screen is a round trip for nothing.
static COVERS: Mutex<Option<std::collections::HashMap<i64, Option<String>>>> = Mutex::new(None);

/// The server's id out of an origin key, `stelly:<id>`.
pub fn track_id(key: &str) -> Option<i64> {
    key.strip_prefix(&format!("{}:", stelly_mix::stelly::SOURCE))?.parse().ok()
}

/// What is already known, without asking: for a first paint that does not
/// wait on the network.
pub fn known_covers(keys: &[String]) -> std::collections::HashMap<String, String> {
    let held = COVERS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(held) = held.as_ref() else { return Default::default() };
    keys.iter()
        .filter_map(|key| {
            let url = held.get(&track_id(key)?)?.clone()?;
            Some((key.clone(), url))
        })
        .collect()
}

/// Fetch whatever of `keys` has no answer yet. A track the server has no
/// art for is remembered as having none, so it is asked for once.
pub fn fetch_covers(keys: &[String]) -> Result<()> {
    let Some(server) = stelly() else { return Ok(()) };
    let wanted: Vec<i64> = {
        let held = COVERS.lock().unwrap_or_else(|e| e.into_inner());
        let known = held.as_ref();
        keys.iter()
            .filter_map(|key| track_id(key))
            .filter(|id| known.is_none_or(|k| !k.contains_key(id)))
            .collect()
    };
    if wanted.is_empty() {
        return Ok(());
    }
    let found = server.covers(&wanted)?;
    let mut held = COVERS.lock().unwrap_or_else(|e| e.into_inner());
    let held = held.get_or_insert_with(Default::default);
    for id in &wanted {
        held.insert(*id, None);
    }
    for (id, url) in found {
        held.insert(id, Some(url));
    }
    Ok(())
}

// ------------------------------------------------------------------- sets

pub fn sets() -> Result<stelly_mix::sets::Sets> {
    stelly_mix::sets::Sets::open(stelly_mix::sets::Sets::default_dir())
}

/// Every set kept, newest first.
pub fn kept_sets() -> Result<Vec<stelly_mix::sets::Set>> {
    sets()?.all()
}

/// Keep `plan` under `name`, as it was planned.
pub fn save_set(loaded: &Loaded, plan: &Plan, name: &str, planned: stelly_mix::sets::Planned) -> Result<stelly_mix::sets::Set> {
    let set = stelly_mix::sets::Set::of(name, plan, &loaded.tracks, planned)?;
    sets()?.put(&set)?;
    Ok(set)
}

/// Re-cut a set after an edit. Nothing is written: an edit is a change to
/// what is on screen, and only Save puts it on disk. The library's own
/// sound scale is handed in rather than worked out again, since working it
/// out samples the whole library.
pub fn edit_set(
    loaded: &Loaded,
    mut set: stelly_mix::sets::Set,
    edit: impl FnOnce(&mut stelly_mix::sets::Set, &[Track], f64, stelly_mix::transitions::SoundScale) -> Result<()>,
) -> Result<stelly_mix::sets::Set> {
    let stretch = set.planned.max_stretch;
    edit(&mut set, &loaded.tracks, stretch, loaded.scale)?;
    Ok(set)
}

/// Plan `set`'s tracklist: from `first` if a track is picked, otherwise
/// from the library's best match for the description it was given.
///
/// The set keeps its name and its id; only what is in it changes, and
/// nothing is written.
pub fn plan_into(loaded: &Loaded, mut set: stelly_mix::sets::Set, first: Option<usize>) -> Result<stelly_mix::sets::Set> {
    let mut options = set.planned.options();
    let wanted = set.planned.like.clone().filter(|text| !text.trim().is_empty());
    if let Some(words) = &wanted {
        options.like = Some(like(loaded, words)?);
    }
    let start = match (first, &options.like) {
        (Some(at), _) => at,
        (None, Some(scores)) => (0..loaded.tracks.len())
            .filter(|&i| loaded.tracks[i].analysis.grid.as_ref().is_some_and(|g| g.steady))
            .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
            .context("the library is empty")?,
        (None, None) => anyhow::bail!("pick a track on the left to start from, or describe how it should sound"),
    };
    let plan = stelly_mix::set::plan(&loaded.tracks, start, &options).context("that track has no tempo to hold a set at")?;
    set.take_plan(&plan, &loaded.tracks)?;
    Ok(set)
}

/// Tracks of the library whose name holds every word of `words`, as the
/// sidebar filters, with how well each would sit in the gap at `at`.
pub fn search_for_gap(
    loaded: &Loaded,
    set: &stelly_mix::sets::Set,
    at: usize,
    words: &str,
    limit: usize,
) -> Vec<(String, String, Option<f64>)> {
    let fits: std::collections::HashMap<String, f64> = set
        .fits_between(at, &loaded.tracks, set.planned.max_stretch, loaded.scale, 400)
        .into_iter()
        .map(|fit| (fit.key, fit.mean))
        .collect();
    let wanted: Vec<String> = words.split_whitespace().map(str::to_lowercase).collect();
    let mut found: Vec<(String, String, Option<f64>)> = loaded
        .tracks
        .iter()
        .filter_map(|track| {
            let key = track.origin.as_ref()?.key();
            if set.tracks.contains(&key) {
                return None;
            }
            let name = track.name();
            let haystack = name.to_lowercase();
            wanted.iter().all(|word| haystack.contains(word.as_str())).then(|| (key.clone(), name, fits.get(&key).copied()))
        })
        .collect();
    // What mixes best first, then the rest by name: a search is for finding
    // one track, but if several match, the one that fits should lead.
    found.sort_by(|a, b| match (a.2, b.2) {
        (Some(x), Some(y)) => y.total_cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
    });
    found.truncate(limit);
    found
}

/// Keep `set` under a new name, as "Save as" does.
pub fn save_set_as(set: &stelly_mix::sets::Set, name: &str) -> Result<stelly_mix::sets::Set> {
    let copy = set.duplicate(name);
    sets()?.put(&copy)?;
    Ok(copy)
}

/// Keep a set up to `at` and plan the rest again. The description it was
/// planned on is scored again where it had one, which wants the text tower.
pub fn replan_set(loaded: &Loaded, mut set: stelly_mix::sets::Set, at: usize) -> Result<stelly_mix::sets::Set> {
    let mut options = set.planned.options();
    if let Some(words) = set.planned.like.clone().filter(|text| !text.trim().is_empty()) {
        options.like = Some(like(loaded, &words)?);
    }
    set.replan_from(at, &loaded.tracks, options)?;
    Ok(set)
}

/// What else could sit at `at` in `set`, best first.
pub fn alternatives_for(loaded: &Loaded, set: &stelly_mix::sets::Set, at: usize, limit: usize) -> Vec<stelly_mix::sets::Fit> {
    set.alternatives(at, &loaded.tracks, set.planned.max_stretch, loaded.scale, limit)
}

/// What would go in the gap before `at`, best first.
pub fn fits_between(loaded: &Loaded, set: &stelly_mix::sets::Set, at: usize, limit: usize) -> Vec<stelly_mix::sets::Fit> {
    set.fits_between(at, &loaded.tracks, set.planned.max_stretch, loaded.scale, limit)
}

pub fn put_set(set: &stelly_mix::sets::Set) -> Result<()> {
    sets()?.put(set)
}

pub fn forget_set(id: &str) -> Result<bool> {
    sets()?.remove(id)
}

/// Write a kept set into a folder to open on the decks.
pub fn export_set(
    set: &stelly_mix::sets::Set,
    loaded: &Loaded,
    into: &Path,
    options: &stelly_mix::export::Options,
    report: &dyn Fn(&str),
) -> Result<stelly_mix::export::Exported> {
    stelly_mix::export::write(set, &loaded.tracks, sources(), into, options, report)
}

/// Every track's likeness to `description`, 0..1 by rank. The text tower is
/// loaded once and kept: it is half a gigabyte.
pub fn like(loaded: &Loaded, description: &str) -> Result<Vec<f64>> {
    static ENCODER: Mutex<Option<TextEncoder>> = Mutex::new(None);
    let mut encoder = ENCODER.lock().unwrap_or_else(|e| e.into_inner());
    if encoder.is_none() {
        let models = stelly_mix::models::dir();
        stelly_mix::models::ensure_text(&models)?;
        *encoder = Some(TextEncoder::load(&models)?);
    }
    stelly_mix::text::scores(encoder.as_mut().expect("just loaded"), &loaded.tracks, description)
}

/// A minute:second clock.
pub fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    format!("{}:{:04.1}", (seconds / 60.0) as u64, seconds % 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_re_cut_is_rendered_to_a_file_of_its_own() {
        let mix = |out, into| Transition { to: 1, out, into, score: 0.5, sound: 0.5, ratio: 1.0, key_steps: None, energy: None, drop_lands: None, cuts_drop: false };
        let paths = [Path::new("/a.mp3"), Path::new("/b.mp3")];
        let settings = Settings::default();
        let name = set_file_name(&paths, &[mix(4, 1)], &settings);
        assert_eq!(name, set_file_name(&paths, &[mix(4, 1)], &settings));
        assert_ne!(name, set_file_name(&paths, &[mix(5, 1)], &settings));
        assert_ne!(name, set_file_name(&paths, &[mix(4, 2)], &settings));
        let other = Settings { keylock: !settings.keylock, ..settings.clone() };
        assert_ne!(name, set_file_name(&paths, &[mix(4, 1)], &other));
    }

    #[test]
    fn peaks_are_each_stretch_loudest_and_scaled_to_one() {
        let samples: Vec<f32> = (0..1000).map(|i| if i == 250 { -0.5 } else if i == 900 { 0.25 } else { 0.0 }).collect();
        let peaks = peaks_of(&samples, 4);
        assert_eq!(peaks, [0.0, 1.0, 0.0, 0.5]);
        assert_eq!(peaks_of(&[], 3), [0.0, 0.0, 0.0]);
        // More buckets than samples still gives as many points as asked.
        assert_eq!(peaks_of(&[0.5, 1.0], 5).len(), 5);
    }

    #[test]
    fn clocks_read_as_minutes_and_seconds() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(83.25), "1:23.2");
        assert_eq!(clock(-3.0), "0:00.0");
    }
}
