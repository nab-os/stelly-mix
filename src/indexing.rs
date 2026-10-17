//! Indexing many tracks at once, for the command line and the app alike:
//! what files, folders and source references come to, then a pool of workers,
//! each with its own CLAP session, that fetch, analyse and store them, saying
//! how it goes as it goes.

use crate::library::{Library, Track};
use crate::source::{Pending, Sources};
use crate::Options;
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use stelly_audio::clap::AudioEncoder;

/// What indexing reads.
pub const AUDIO: [&str; 8] = ["mp3", "flac", "wav", "aif", "aiff", "ogg", "opus", "m4a"];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO.contains(&e.to_ascii_lowercase().as_str()))
}

/// Every file under `dir`, depth first, in no particular order.
pub fn walk(dir: &Path, each: &mut dyn FnMut(&Path)) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, each)?;
        } else {
            each(&path);
        }
    }
    Ok(())
}

/// What `arguments` come to: a folder's every file, a file itself, a
/// source's reference what it resolves to.
pub fn gather<'a>(arguments: &[String], sources: &'a Sources) -> Result<Vec<Pending<'a>>> {
    let mut files = Vec::new();
    for argument in arguments {
        let path = Path::new(argument);
        if path.is_dir() {
            walk(path, &mut |p| files.push(Pending::File(p.to_path_buf())))?;
        } else {
            files.extend(sources.pending(argument)?);
        }
    }
    Ok(files)
}

/// How indexing is going.
pub enum Event<'a> {
    /// About to start on this many; none at all is everything indexed already.
    Starting { total: usize },
    Indexed { done: usize, total: usize, track: &'a Track },
    Failed { name: String, error: String },
}

/// Index what is not indexed yet among `files`, in `options`' octave, into
/// `library`; the number indexed. What is indexed already is passed over, a
/// source's track by where it came from, and the rest is fetched by the
/// workers, so a playlist is analysed as it comes down rather than after the
/// last of it has. A source's track is fetched as light as analysis allows,
/// and what was fetched for it goes once it is analysed, unless `keep_audio`:
/// `Sources::audio` brings it back to be heard.
pub fn run(
    mut files: Vec<Pending<'_>>,
    library: &Library,
    options: &Options,
    keep_audio: bool,
    report: &(dyn Fn(Event) + Sync),
) -> Result<usize> {
    // Indexed in another octave is not indexed: a new range asks again.
    let in_range = |file: &Path| library.get(file).is_some_and(|t| t.bpm_range == options.bpm_range);
    let had = |key: &str| library.by_origin(key).is_some_and(|t| t.bpm_range == options.bpm_range);
    files.retain(|p| match p.origin() {
        Some(origin) => !had(&origin.key()),
        None => p.cached().is_none_or(|f| is_audio(&f) && !in_range(&f)),
    });
    files.sort_by_key(|p| match p {
        Pending::File(f) => Some(f.clone()),
        Pending::Item(..) => None,
    });
    report(Event::Starting { total: files.len() });
    if files.is_empty() {
        return Ok(0);
    }
    let models = crate::models::dir();
    crate::models::ensure(&models)?;

    // Each worker holds an ONNX session; a few already keep every core busy.
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get() / 2)
        .clamp(1, 8)
        .min(files.len());
    let (next, done) = (AtomicUsize::new(0), AtomicUsize::new(0));
    std::thread::scope(|scope| -> Result<()> {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    let mut encoder = AudioEncoder::load(&models, 1)?;
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(pending) = files.get(i) else { return Ok(()) };
                        let fetched_now = pending.cached().is_none();
                        let file = match pending.fetch_for_analysis() {
                            Ok(file) => file,
                            Err(err) => {
                                report(Event::Failed { name: pending.name(), error: format!("{err:#}") });
                                continue;
                            }
                        };
                        let origin = pending.origin();
                        // A fetched file's name is its source's: checked once it is here.
                        if origin.is_some() && !is_audio(&file) {
                            report(Event::Failed { name: pending.name(), error: format!("{} is no audio", file.display()) });
                            continue;
                        }
                        let analysed = crate::library::index(&file, options, &mut encoder);
                        if origin.is_some() && fetched_now && !keep_audio {
                            let _ = std::fs::remove_file(&file);
                        }
                        match analysed {
                            Ok(mut track) => {
                                track.origin = origin;
                                library.put(&track)?;
                                let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                                report(Event::Indexed { done, total: files.len(), track: &track });
                            }
                            Err(err) => report(Event::Failed {
                                name: pending.name(),
                                error: format!("{err:#}"),
                            }),
                        }
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("an indexing worker panicked")?;
        }
        Ok(())
    })?;
    Ok(done.into_inner())
}
