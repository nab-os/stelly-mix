//! Tempo candidates of a file, and how well the beat tracker and grid do
//! seeded with each, plus any tempos given after the file. A diagnostic.

use stelly_audio::descriptors;
use stelly_mix::{beats, grid, tempo};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("a file");
    let pcm = stelly_mix::decode::decode_file(path.as_ref())?;
    let onsets = descriptors::onsets(&pcm.samples, pcm.rate);
    let peaks: Vec<f64> = tempo::candidates(&onsets);
    println!("described bpm {:?}", descriptors::extract(&pcm.samples, pcm.rate)?.bpm);
    let seeds: Vec<f64> = std::env::args().skip(2).map(|a| a.parse().unwrap()).collect();
    for bpm in peaks.iter().chain(&seeds) {
        let tracked = beats::track(&onsets, *bpm);
        let fit = grid::fit(&tracked);
        println!(
            "{bpm:7.2} BPM  -> {} beats, grid {:?} BPM, {:.0}% on grid",
            tracked.len(),
            fit.as_ref().map(|g| (g.bpm * 100.0).round() / 100.0),
            fit.as_ref().map_or(0.0, |g| g.on_grid * 100.0)
        );
    }
    Ok(())
}
