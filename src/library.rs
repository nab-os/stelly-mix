//! The tracks indexed so far: each one's analysis and segments, one JSON file
//! a track under `$XDG_DATA_HOME/stelly-mix/library`, redone when the file
//! changes or the analysis does.

use crate::segments::{self, Segment};
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
    pub analysis: Analysis,
    pub segments: Vec<Segment>,
}

impl Track {
    pub fn name(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn bpm(&self) -> Option<f64> {
        self.analysis.grid.as_ref().map(|g| g.bpm)
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
    /// changed since.
    pub fn get(&self, path: &Path) -> Option<Track> {
        let path = path.canonicalize().ok()?;
        let text = std::fs::read_to_string(self.entry(&path)).ok()?;
        let track: Track = serde_json::from_str(&text).ok()?;
        let (size, modified) = stamp(&path).ok()?;
        (track.version == VERSION && track.size == size && track.modified == modified).then_some(track)
    }

    pub fn put(&self, track: &Track) -> Result<()> {
        let entry = self.entry(&track.path);
        let partial = entry.with_extension("part");
        std::fs::write(&partial, serde_json::to_vec(track)?)?;
        std::fs::rename(&partial, &entry)?;
        Ok(())
    }

    /// Every current entry. Ones from an older analysis are left out.
    pub fn all(&self) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            if let Ok(track) = serde_json::from_str::<Track>(&text) {
                if track.version == VERSION && track.path.exists() {
                    tracks.push(track);
                }
            }
        }
        tracks.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(tracks)
    }

    fn entry(&self, canonical: &Path) -> PathBuf {
        let digest = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
        let name: String = digest[..10].iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{name}.json"))
    }
}

/// Decode, analyse and cut one file.
pub fn index(path: &Path, options: &Options, encoder: &mut AudioEncoder) -> Result<Track> {
    let path = path.canonicalize().with_context(|| format!("finding {}", path.display()))?;
    let (size, modified) = stamp(&path)?;
    let pcm = crate::decode::decode_file(&path)?;
    let analysis = crate::analyse(&pcm, options)?;
    let segments = segments::cut(&pcm, &analysis, encoder)?;
    Ok(Track {
        path,
        size,
        modified,
        version: VERSION,
        analysis,
        segments,
    })
}

fn stamp(path: &Path) -> Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    let modified = meta.modified()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok((meta.len(), modified))
}
