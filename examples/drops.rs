//! Where the drops are, with each bar's low end to check them against: a
//! file, or a library track by part of its name, fetched again if need be.

use stelly_mix::library::Library;
use stelly_mix::source::Sources;
use stelly_mix::{bands, drops, structure};

fn main() -> anyhow::Result<()> {
    let library = Library::open(Library::default_dir())?;
    let sources = Sources::new(stelly_mix::stelly::sources());
    let tracks = library.all()?;
    for argument in std::env::args().skip(1) {
        let file = if std::path::Path::new(&argument).is_file() {
            argument.into()
        } else {
            let wanted = argument.to_lowercase();
            let track = tracks.iter().find(|t| t.name().to_lowercase().contains(&wanted)).expect("no such track");
            sources.audio(track)?
        };
        let pcm = stelly_mix::decode::decode_file(&file)?;
        let analysis = stelly_mix::analyse(&pcm, &Default::default())?;
        let features = bands::raw_per_beat(&pcm.samples, pcm.rate, &analysis.beats);
        let downbeat = analysis.downbeats.first().and_then(|d| analysis.beats.iter().position(|b| b == d)).unwrap_or(0);
        let low = drops::low_per_bar(&features, downbeat);
        let found = drops::find(&low);

        println!("\n{}", file.display());
        for (phrase, bars) in low.chunks(structure::PHRASE_BARS).enumerate() {
            let first = phrase * structure::PHRASE_BARS;
            let row: Vec<String> = bars
                .iter()
                .enumerate()
                .map(|(i, v)| format!("{}{v:5.1}", if found.contains(&(first + i)) { "▶" } else { " " }))
                .collect();
            println!("  {}  {}", clock(analysis.downbeats[first]), row.join(""));
        }
        let frames = bands::Frames::of(&pcm.samples, pcm.rate);
        let at: Vec<String> = drops::at(&frames, &analysis.beats, downbeat).into_iter().map(clock).collect();
        println!("  drops at {}", at.join(", "));
    }
    Ok(())
}

fn clock(seconds: f64) -> String {
    format!("{}:{:05.2}", (seconds / 60.0) as u64, seconds % 60.0)
}
