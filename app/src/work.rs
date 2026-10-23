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
    let names: Vec<String> = chain.iter().map(|t| t.path.to_string_lossy().into_owned()).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let path = dir.join(file_name(&refs));
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

/// Re-cut a set after an edit and keep it. The library's own sound scale is
/// handed in rather than worked out again: it samples the whole library.
pub fn edit_set(
    loaded: &Loaded,
    mut set: stelly_mix::sets::Set,
    edit: impl FnOnce(&mut stelly_mix::sets::Set, &[Track], f64, stelly_mix::transitions::SoundScale) -> Result<()>,
) -> Result<stelly_mix::sets::Set> {
    let stretch = set.planned.max_stretch;
    edit(&mut set, &loaded.tracks, stretch, loaded.scale)?;
    sets()?.put(&set)?;
    Ok(set)
}

/// Keep a set up to `at` and plan the rest again. The description it was
/// planned on is scored again where it had one, which wants the text tower.
pub fn replan_set(loaded: &Loaded, mut set: stelly_mix::sets::Set, at: usize) -> Result<stelly_mix::sets::Set> {
    let mut options = set.planned.options();
    if let Some(words) = set.planned.like.clone().filter(|text| !text.trim().is_empty()) {
        options.like = Some(like(loaded, &words)?);
    }
    set.replan_from(at, &loaded.tracks, options)?;
    sets()?.put(&set)?;
    Ok(set)
}

/// What else could sit at `at` in `set`, best first.
pub fn alternatives_for(loaded: &Loaded, set: &stelly_mix::sets::Set, at: usize, limit: usize) -> Vec<(String, f64)> {
    set.alternatives(at, &loaded.tracks, set.planned.max_stretch, loaded.scale, limit)
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
