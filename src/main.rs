//! The command line: analyse files and print what a DJ would want to know.

use anyhow::Result;
use argh::FromArgs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(FromArgs)]
/// Beat grids, phrases and keys for people who mix.
struct Cli {
    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Analyse(Analyse),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "analyse")]
/// Beat grid, downbeats, phrases and key of each file.
struct Analyse {
    /// print one JSON object per file, every beat included
    #[argh(switch)]
    json: bool,
    /// audio files: MP3, FLAC, WAV, AIFF, Ogg, M4A
    #[argh(positional)]
    files: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let cli: Cli = argh::from_env();
    match cli.command {
        Command::Analyse(args) => {
            let mut failed = 0;
            for file in &args.files {
                if let Err(err) = analyse(file, args.json) {
                    eprintln!("{}: {err:#}", file.display());
                    failed += 1;
                }
            }
            if failed > 0 {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

fn analyse(file: &Path, json: bool) -> Result<()> {
    let started = Instant::now();
    let pcm = stelly_audio::decode::decode_file(file)?;
    let analysis = stelly_mix::analyse(&pcm)?;

    if json {
        let mut value = serde_json::to_value(&analysis)?;
        value["file"] = file.display().to_string().into();
        println!("{value}");
        return Ok(());
    }

    println!("{}", file.display());
    match &analysis.grid {
        Some(grid) => println!(
            "  {:.2} BPM, {} ({:.0}% of beats on the grid), first beat at {:.3}s",
            grid.bpm,
            if grid.steady { "steady" } else { "drifting" },
            grid.on_grid * 100.0,
            grid.first_beat
        ),
        None => println!("  no beat grid, {} beats tracked", analysis.beats.len()),
    }
    if let Some(key) = &analysis.key {
        println!("  {} ({} {}, strength {:.2})", key.camelot, key.tonic, key.scale, key.strength);
    }
    if let Some(first) = analysis.downbeats.first() {
        println!(
            "  first downbeat at {first:.3}s (confidence {:.2}), {} bars, {} phrases",
            analysis.downbeat_confidence,
            analysis.downbeats.len(),
            analysis.phrases.len()
        );
    }
    if !analysis.sections.is_empty() {
        let sections: Vec<String> = analysis.sections.iter().map(|&s| clock(s)).collect();
        println!("  sections at {}", sections.join(", "));
    }
    println!(
        "  {} long, analysed in {:.1}s",
        clock(analysis.seconds),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn clock(seconds: f64) -> String {
    format!("{}:{:04.1}", (seconds / 60.0) as u64, seconds % 60.0)
}
