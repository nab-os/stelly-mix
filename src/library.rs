//! The tracks indexed so far: each one's analysis and segments, one JSON file
//! a track under `$XDG_DATA_HOME/stelly-mix/library`, redone when the file
//! changes or the analysis does.
//!
//! A track from a source is kept by where it came from rather than by its
//! file, which can go once it is analysed: `Sources::audio` fetches it again.

use crate::segments::{self, Segment};
use crate::source::Origin;
use crate::{Analysis, Options};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use stelly_audio::clap::AudioEncoder;

/// Bump whenever the analysis or the segments would come out differently.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,
    pub size: u64,
    /// Seconds since the epoch.
    pub modified: u64,
    pub version: u32,
    /// The octave its tempo was reported in; entries from before this was
    /// kept were all indexed in the default one.
    #[serde(default = "default_bpm_range")]
    pub bpm_range: (f64, f64),
    /// Where it came from when not a file of its own; its `path` is then the
    /// file it was analysed from, which may be gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    pub analysis: Analysis,
    pub segments: Vec<Segment>,
    /// Whether there is a voice in it, phrase by phrase, when it has been
    /// listened for. Absent on an entry written before it was, and on one
    /// nobody has run `vocals` over yet: that is a track nothing is known
    /// about, not a track with no singing in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocals: Option<crate::vocals::Heard>,
    /// What tells one recording from another, worked out once when asked.
    #[serde(skip)]
    pub fingerprint: std::sync::OnceLock<Fingerprint>,
}

/// What `transitions::same_recording` compares: a name with track numbers
/// and punctuation set aside, a length, and how the whole track sounds.
#[derive(Debug, Clone)]
pub struct Fingerprint {
    pub name: String,
    pub seconds: f64,
    /// The phrases' embeddings averaged and normalised.
    pub sound: Vec<f32>,
}

fn default_bpm_range() -> (f64, f64) {
    crate::tempo::DJ_BPM
}

impl Track {
    pub fn name(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn bpm(&self) -> Option<f64> {
        self.analysis.grid.as_ref().map(|g| g.bpm)
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        self.fingerprint.get_or_init(|| {
            let name: String = self.name().to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
            let mut sound = vec![0f32; self.segments.first().map_or(0, |s| s.embedding.len())];
            for segment in &self.segments {
                for (m, v) in sound.iter_mut().zip(&segment.embedding) {
                    *m += v;
                }
            }
            let norm = sound.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
            sound.iter_mut().for_each(|v| *v /= norm);
            Fingerprint {
                name: name.trim_start_matches(|c: char| c.is_ascii_digit()).to_string(),
                seconds: self.analysis.seconds,
                sound,
            }
        })
    }
}

pub struct Library {
    dir: PathBuf,
}

impl Library {
    pub fn open(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn default_dir() -> PathBuf {
        crate::models::data_home().join("stelly-mix/library")
    }

    /// The stored entry for `path`, if there is one and the file has not
    /// changed since, whatever octave it was indexed in.
    pub fn get(&self, path: &Path) -> Option<Track> {
        let path = path.canonicalize().ok()?;
        let text = std::fs::read_to_string(self.entry(&path)).ok()?;
        let track: Track = serde_json::from_str(&text).ok()?;
        current(&track).then_some(track)
    }

    /// The entry for a track from a source, by its origin's key.
    pub fn by_origin(&self, key: &str) -> Option<Track> {
        let text = std::fs::read_to_string(self.dir.join(entry_name(key.as_bytes()))).ok()?;
        let track: Track = serde_json::from_str(&text).ok()?;
        (track.origin.as_ref().is_some_and(|o| o.key() == key) && current(&track)).then_some(track)
    }

    /// Analyse `file`, fetched from `origin`, into the library, in the
    /// default octave.
    pub fn index_origin(&self, file: &Path, origin: Origin) -> Result<Track> {
        let models = crate::models::dir();
        crate::models::ensure(&models)?;
        let mut encoder = AudioEncoder::load(&models, 4)?;
        let mut track = index(file, &Options::default(), &mut encoder)?;
        track.origin = Some(origin);
        self.put(&track)?;
        Ok(track)
    }

    /// The entry for `file`, indexing it first if need be, in the default
    /// octave.
    pub fn indexed(&self, file: &Path) -> Result<Track> {
        if let Some(track) = self.get(file) {
            return Ok(track);
        }
        let models = crate::models::dir();
        crate::models::ensure(&models)?;
        let mut encoder = AudioEncoder::load(&models, 4)?;
        let track = index(file, &Options::default(), &mut encoder)?;
        self.put(&track)?;
        Ok(track)
    }

    pub fn put(&self, track: &Track) -> Result<()> {
        let entry = match &track.origin {
            Some(origin) => self.dir.join(entry_name(origin.key().as_bytes())),
            None => self.entry(&track.path),
        };
        // Written aside under a name of its own, so two writers, two indexing
        // runs say, never write into one file, then moved into place whole.
        static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial = entry.with_extension(format!("{}-{n}.part", std::process::id()));
        std::fs::write(&partial, serde_json::to_vec(track)?)?;
        std::fs::rename(&partial, &entry)?;
        Ok(())
    }

    /// Every current entry. Ones from an older analysis, or for a file that
    /// has changed or gone since, are left out, as `get` leaves them. Read
    /// on every core: a large library is tens of thousands of files.
    pub fn all(&self) -> Result<Vec<Track>> {
        let paths = self.entries()?;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
        let mut tracks: Vec<Track> = std::thread::scope(|scope| {
            let handles: Vec<_> = paths
                .chunks(paths.len().div_ceil(threads).max(1))
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .filter_map(|path| serde_json::from_slice::<Track>(&std::fs::read(path).ok()?).ok())
                            .filter(current)
                            .collect::<Vec<Track>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("a library reader panicked")).collect()
        });
        tracks.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(tracks)
    }

    /// Rewrite every entry in the current form, embeddings compact: the
    /// number rewritten.
    pub fn compact(&self) -> Result<usize> {
        let mut done = 0;
        for path in self.entries()? {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            // Already compact: its embeddings are strings.
            if !text.contains("\"embedding\":[") {
                continue;
            }
            if let Ok(track) = serde_json::from_str::<Track>(&text) {
                let partial = path.with_extension("compact.part");
                std::fs::write(&partial, serde_json::to_vec(&track)?)?;
                std::fs::rename(&partial, &path)?;
                done += 1;
            }
        }
        Ok(done)
    }

    /// Call `from`'s tracks `to`'s: each entry fetched from the source named
    /// `from` is filed under `to` instead, the same item. The number moved.
    pub fn rename_source(&self, from: &str, to: &str) -> Result<usize> {
        let mut done = 0;
        for path in self.entries()? {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Ok(mut track) = serde_json::from_slice::<Track>(&bytes) else { continue };
            let Some(origin) = track.origin.as_mut().filter(|o| o.source == from) else { continue };
            origin.source = to.to_string();
            self.put(&track)?;
            std::fs::remove_file(&path)?;
            done += 1;
        }
        Ok(done)
    }

    /// Bring entries filed under any of `from` under `to`, once. Mirrors
    /// made before a source was renamed hold the same tracks under the old
    /// name; left there they are fetched a second time, under the new one.
    /// The marker means a library already adopted is not scanned again.
    pub fn adopt_sources(&self, from: &[&str], to: &str) -> Result<usize> {
        let marker = self.dir.join(format!(".adopted-{to}"));
        if marker.exists() {
            return Ok(0);
        }
        let mut done = 0;
        for source in from {
            done += self.rename_source(source, to)?;
        }
        std::fs::write(&marker, [])?;
        Ok(done)
    }

    /// Listen through the library for voices, and keep what was heard in
    /// each entry: how many were listened to.
    ///
    /// Done over the whole library at once because the text tower is half a
    /// gigabyte to load and the scoring itself is a dot product; and kept,
    /// because nothing about a track changes to make it worth hearing twice.
    pub fn listen(&self, report: &dyn Fn(usize, usize)) -> Result<usize> {
        let tracks = self.all()?;
        let todo: Vec<Track> = tracks
            .into_iter()
            .filter(|track| track.vocals.as_ref().is_none_or(|heard| heard.version != crate::vocals::VERSION))
            .filter(|track| !track.segments.is_empty())
            .collect();
        report(0, todo.len());
        if todo.is_empty() {
            return Ok(0);
        }
        let ear = crate::vocals::Ear::load()?;
        let mut done = 0;
        for mut track in todo {
            track.vocals = Some(ear.track(&track));
            self.put(&track)?;
            done += 1;
            if done % 50 == 0 {
                report(done, done);
            }
        }
        Ok(done)
    }

    fn entries(&self) -> Result<Vec<PathBuf>> {
        Ok(std::fs::read_dir(&self.dir)?
            .filter_map(|e| Some(e.ok()?.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect())
    }

    fn entry(&self, canonical: &Path) -> PathBuf {
        self.dir.join(entry_name(canonical.as_os_str().as_encoded_bytes()))
    }
}

/// An entry's file name: a path's hash for a file, an origin's for a source's track.
fn entry_name(key: &[u8]) -> String {
    let digest = Sha256::digest(key);
    let name: String = digest[..10].iter().map(|b| format!("{b:02x}")).collect();
    format!("{name}.json")
}

/// Whether an entry still holds: this analysis, and the file as it was. A
/// source's track is the source's recording, however its file comes and goes.
fn current(track: &Track) -> bool {
    track.version == VERSION
        && (track.origin.is_some() || stamp(&track.path).is_ok_and(|stamp| stamp == (track.size, track.modified)))
}

/// Decode, analyse and cut one file.
pub fn index(path: &Path, options: &Options, encoder: &mut AudioEncoder) -> Result<Track> {
    let path = path.canonicalize().with_context(|| format!("finding {}", path.display()))?;
    let (size, modified) = stamp(&path)?;
    let pcm = crate::decode::decode_file(&path)?;
    let mut track = index_pcm(&pcm, path, options, encoder)?;
    (track.size, track.modified) = (size, modified);
    Ok(track)
}

/// Analyse and cut audio already decoded, which came from `origin`: a
/// server's, fetched to memory rather than to a file. Its `path` is what it
/// is called, a stand-in for a file it never had.
pub fn index_origin_pcm(pcm: &stelly_audio::decode::Pcm, origin: Origin, options: &Options, encoder: &mut AudioEncoder) -> Result<Track> {
    let name = PathBuf::from(origin.item.file_stem());
    let mut track = index_pcm(pcm, name, options, encoder)?;
    track.origin = Some(origin);
    Ok(track)
}

fn index_pcm(pcm: &stelly_audio::decode::Pcm, path: PathBuf, options: &Options, encoder: &mut AudioEncoder) -> Result<Track> {
    let analysis = crate::analyse(pcm, options)?;
    let segments = segments::cut(pcm, &analysis, encoder)?;
    Ok(Track {
        path,
        size: 0,
        modified: 0,
        version: VERSION,
        bpm_range: options.bpm_range,
        origin: None,
        analysis,
        segments,
        vocals: None,
        fingerprint: Default::default(),
    })
}

fn stamp(path: &Path) -> Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    let modified = meta.modified()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok((meta.len(), modified))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_changed_since_is_not_current() {
        let dir = std::env::temp_dir().join(format!("stelly-mix-library-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("track.flac");
        std::fs::write(&file, b"before").unwrap();
        let (size, modified) = stamp(&file).unwrap();
        let mut track: Track = serde_json::from_value(serde_json::json!({
            "path": file, "size": size, "modified": modified, "version": VERSION,
            "analysis": {
                "seconds": 1.0, "grid": null, "key": null, "loudness": null, "beats": [],
                "downbeats": [], "downbeat_confidence": 0.0, "phrases": [], "sections": []
            },
            "segments": []
        }))
        .unwrap();
        // An entry from before the range was kept was indexed in the default one.
        assert_eq!(track.bpm_range, crate::tempo::DJ_BPM);
        assert!(current(&track));

        std::fs::write(&file, b"after, and longer").unwrap();
        assert!(!current(&track), "a changed file is stale");
        track.size = std::fs::metadata(&file).unwrap().len();
        track.modified = stamp(&file).unwrap().1;
        assert!(current(&track));
        std::fs::remove_file(&file).unwrap();
        assert!(!current(&track), "a gone file is stale");

        // A source's track outlives its file, and is found by where it came from.
        let library = Library::open(dir.join("library")).unwrap();
        track.origin = Some(Origin {
            source: "qobuz".into(),
            item: crate::source::Item { id: "42".into(), artist: "A".into(), title: "T".into(), seconds: None },
        });
        assert!(current(&track));
        library.put(&track).unwrap();
        assert_eq!(library.by_origin("qobuz:42").map(|t| t.path), Some(file.clone()));
        assert!(library.by_origin("qobuz:43").is_none());

        // Filed under another source's name, the same item.
        assert_eq!(library.rename_source("qobuz", "stelly").unwrap(), 1);
        assert!(library.by_origin("qobuz:42").is_none());
        assert_eq!(library.by_origin("stelly:42").map(|t| t.path), Some(file.clone()));
        assert_eq!(library.rename_source("qobuz", "stelly").unwrap(), 0);
        assert_eq!(library.all().unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
