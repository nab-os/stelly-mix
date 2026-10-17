//! Finding tracks by description: CLAP's text tower puts "chill downtempo"
//! in the same space as the phrases, so a track's phrases can be scored
//! against it. Ported from Stelly's server, same weights.
//!
//! Raw similarity to one phrase of text mostly measures how much a track
//! sounds like music at all, which Stelly found the hard way with its moods.
//! So a track is scored by how much nearer the description its phrases are
//! than to plain "music", and the scores are then ranked across the library.

use crate::library::Track;
use anyhow::{Context, Result};
use ort::session::Session;
use ort::value::TensorRef;
use std::path::Path;
use stelly_audio::clap::{normalise, CLAP_TEXT, CLAP_TOKENIZER};
use tokenizers::Tokenizer;

/// What a description is measured against.
const BASELINE: &str = "music";

pub struct TextEncoder {
    session: Session,
    tokenizer: Tokenizer,
}

impl TextEncoder {
    pub fn load(model_dir: &Path) -> Result<Self> {
        let onnx = model_dir.join(CLAP_TEXT);
        let session = Session::builder()
            .context("creating an ONNX session")?
            .commit_from_file(&onnx)
            .with_context(|| format!("loading {}", onnx.display()))?;
        let tokenizer = Tokenizer::from_file(model_dir.join(CLAP_TOKENIZER))
            .map_err(|e| anyhow::anyhow!("loading the tokenizer: {e}"))?;
        Ok(Self { session, tokenizer })
    }

    /// L2-normalised 512-d embedding. One text a call: the export takes no
    /// attention mask, so a padded batch would embed the padding too.
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenising {text:?}: {e}"))?;
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&v| v as i64).collect();
        let shape = [1_i64, ids.len() as i64];
        let outputs = self.session.run(ort::inputs![
            "input_ids" => TensorRef::from_array_view((shape, ids.as_slice()))?,
        ])?;
        let (_, data) = outputs["text_embeds"].try_extract_tensor::<f32>()?;
        let mut vector = data.to_vec();
        normalise(&mut vector);
        Ok(vector)
    }
}

/// How much each track sounds like `description`, as its rank across the
/// library: 1 the most, 0 the least. A track is its phrases' mean, so a
/// chill intro does not make a banger chill.
pub fn scores(encoder: &mut TextEncoder, tracks: &[Track], description: &str) -> Result<Vec<f64>> {
    let query = encoder.embed(description)?;
    let baseline = encoder.embed(BASELINE)?;
    let raw: Vec<f64> = tracks
        .iter()
        .map(|track| {
            let n = track.segments.len().max(1) as f64;
            track
                .segments
                .iter()
                .map(|s| dot(&s.embedding, &query) - dot(&s.embedding, &baseline))
                .sum::<f64>()
                / n
        })
        .collect();
    let mut order: Vec<usize> = (0..raw.len()).collect();
    order.sort_by(|&a, &b| raw[a].total_cmp(&raw[b]));
    let mut ranks = vec![0.0; raw.len()];
    for (rank, &i) in order.iter().enumerate() {
        ranks[i] = rank as f64 / (raw.len().max(2) - 1) as f64;
    }
    Ok(ranks)
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x * y) as f64).sum()
}
