//! Every tempo candidate of each file, with what a selection rule might
//! weigh, one JSON object per file: for trying rules without rebuilding.

use stelly_audio::descriptors;
use stelly_mix::{beats, grid, tempo};

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(pcm) = stelly_mix::decode::decode_file(path.as_ref()) else { continue };
        let onsets = descriptors::onsets(&pcm.samples, pcm.rate);
        let Ok(described) = descriptors::extract(&pcm.samples, pcm.rate) else { continue };
        let mean = onsets.envelope.iter().sum::<f64>() / onsets.envelope.len() as f64;
        let rows: Vec<serde_json::Value> = tempo::candidates(&onsets)
            .into_iter()
            .chain(described.bpm)
            .map(|bpm| {
                let tracked = beats::track(&onsets, bpm);
                let fit = grid::fit(&tracked);
                let salience = tracked.iter().map(|&b| beats::strength(&onsets, b)).sum::<f64>()
                    / tracked.len().max(1) as f64
                    / mean;
                serde_json::json!({
                    "seed": bpm,
                    "bpm": fit.as_ref().map(|g| g.bpm),
                    "share": fit.as_ref().map_or(0.0, |g| g.on_grid),
                    "salience": salience,
                })
            })
            .collect();
        println!("{}", serde_json::json!({ "file": path, "candidates": rows }));
    }
}
