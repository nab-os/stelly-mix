//! A grid fitted per 20s window, seeded at a given tempo: does it change?

use stelly_audio::descriptors;
use stelly_mix::{beats, grid};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a file");
    let bpm: f64 = args.next().expect("a tempo").parse()?;
    let pcm = stelly_mix::decode::decode_file(path.as_ref())?;
    let tracked = beats::track(&descriptors::onsets(&pcm.samples, pcm.rate), bpm);
    let mut start = 0.0;
    while start < pcm.seconds() {
        let window: Vec<f64> = tracked.iter().copied().filter(|&b| b >= start && b < start + 20.0).collect();
        match grid::fit(&window) {
            Some(g) => println!("{start:5.0}s  {:3} beats  {:7.2} BPM  {:3.0}% on grid", window.len(), g.bpm, g.on_grid * 100.0),
            None => println!("{start:5.0}s  {:3} beats", window.len()),
        }
        start += 20.0;
    }
    Ok(())
}
