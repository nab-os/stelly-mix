//! CLAP's weights, fetched once and checked by hash: the same pinned export
//! Stelly runs, so a phrase embedded here lands in the same space as a track
//! embedded there, and a description in the same space as both.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use stelly_audio::clap::{CLAP_AUDIO, CLAP_TEXT, CLAP_TOKENIZER};

const REPO: &str = "https://huggingface.co/Xenova/clap-htsat-unfused/resolve/c28f2883575e590e04d3146ff0713c2448d691ba";

struct Weights {
    name: &'static str,
    remote: &'static str,
    bytes: u64,
    sha256: &'static str,
}

const WEIGHTS: [Weights; 3] = [
    Weights {
        name: CLAP_AUDIO,
        remote: "onnx/audio_model.onnx",
        bytes: 117_528_416,
        sha256: "a1c2b43c44f71e0fa841a4b86700886c199bf87699ea45632c4d831bc6c88957",
    },
    Weights {
        name: CLAP_TEXT,
        remote: "onnx/text_model.onnx",
        bytes: 501_513_769,
        sha256: "6df7c51f2d78e236a03f2b816e613d538c815639d13644fe7c2124f439da9648",
    },
    Weights {
        name: CLAP_TOKENIZER,
        remote: "tokenizer.json",
        bytes: 2_108_774,
        sha256: "dc239041d98de27ffc3975473a1a23e3db4c937b23c138c38bbc66588bd247e5",
    },
];

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
    ensure_named(dir, &[CLAP_AUDIO])
}

/// The text tower and its tokenizer, fetched if they are not there yet.
pub fn ensure_text(dir: &Path) -> Result<()> {
    ensure_named(dir, &[CLAP_TEXT, CLAP_TOKENIZER])
}

fn ensure_named(dir: &Path, names: &[&str]) -> Result<()> {
    for weights in WEIGHTS.iter().filter(|w| names.contains(&w.name)) {
        let target = dir.join(weights.name);
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == weights.bytes) {
            continue;
        }
        fetch(dir, weights).with_context(|| format!("fetching {}", weights.name))?;
    }
    Ok(())
}

fn fetch(dir: &Path, weights: &Weights) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    eprintln!("fetching {} ({:.0} MB) into {}", weights.name, weights.bytes as f64 / 1e6, dir.display());
    let url = format!("{REPO}/{}", weights.remote);
    let mut response = ureq::get(&url).call()?;
    let mut body = response.body_mut().with_config().limit(weights.bytes + 1).reader();
    let target = dir.join(weights.name);
    let partial = target.with_extension("part");
    let mut file = std::fs::File::create(&partial)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = body.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        file.write_all(&buffer[..n])?;
    }
    file.flush()?;
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if digest != weights.sha256 {
        let _ = std::fs::remove_file(&partial);
        bail!("came down with hash {digest}, expected {}", weights.sha256);
    }
    std::fs::rename(&partial, &target)?;
    Ok(())
}
