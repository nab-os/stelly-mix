//! CLAP's weights, fetched once and checked by hash: the same pinned export
//! Stelly runs, so a phrase embedded here lands in the same space as a track
//! embedded there, and a description in the same space as both. What they
//! are and how they are fetched is stelly-audio's; where they go is here.

use anyhow::Result;
use std::path::{Path, PathBuf};
use stelly_audio::models::{self, Weights};

/// `$STELLY_MIX_MODELS`, or `$XDG_DATA_HOME/stelly-mix/models`.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("STELLY_MIX_MODELS") {
        return dir.into();
    }
    data_home().join("stelly-mix/models")
}

pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
}

/// The audio tower, fetched if it is not there yet.
pub fn ensure(dir: &Path) -> Result<()> {
    models::ensure(dir, models::AUDIO, announce(dir))
}

/// The text tower and its tokenizer, fetched if they are not there yet.
pub fn ensure_text(dir: &Path) -> Result<()> {
    models::ensure(dir, models::TEXT, announce(dir))
}

fn announce(dir: &Path) -> impl FnMut(&Weights) + '_ {
    move |weights| eprintln!("fetching {} ({:.0} MB) into {}", weights.name, weights.bytes as f64 / 1e6, dir.display())
}
