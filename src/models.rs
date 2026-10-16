//! CLAP's audio tower, fetched once and checked by hash: the same pinned
//! export Stelly runs, so a phrase embedded here lands in the same space as a
//! track embedded there.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use stelly_audio::clap::CLAP_AUDIO;

const URL: &str = "https://huggingface.co/Xenova/clap-htsat-unfused/resolve/\
                   c28f2883575e590e04d3146ff0713c2448d691ba/onnx/audio_model.onnx";
const BYTES: u64 = 117_528_416;
const SHA256: &str = "a1c2b43c44f71e0fa841a4b86700886c199bf87699ea45632c4d831bc6c88957";

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

/// The weights' directory, after fetching them if they are not there yet.
pub fn ensure(dir: &Path) -> Result<()> {
    let target = dir.join(CLAP_AUDIO);
    if std::fs::metadata(&target).is_ok_and(|m| m.len() == BYTES) {
        return Ok(());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    eprintln!("fetching CLAP's audio tower ({:.0} MB) into {}", BYTES as f64 / 1e6, dir.display());

    let mut response = ureq::get(URL).call().context("fetching CLAP's weights")?;
    let mut body = response.body_mut().with_config().limit(BYTES + 1).reader();
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
    if digest != SHA256 {
        let _ = std::fs::remove_file(&partial);
        bail!("CLAP's weights came down with hash {digest}, expected {SHA256}");
    }
    std::fs::rename(&partial, &target)?;
    Ok(())
}
