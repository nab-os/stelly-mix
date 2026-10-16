//! The command line: analyse files and print what a DJ would want to know.

use anyhow::{Context, Result};
use argh::FromArgs;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use stelly_mix::eval::{self, Truth};

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
    Eval(Eval),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "analyse")]
/// Beat grid, downbeats, phrases and key of each file.
struct Analyse {
    /// print one JSON object per file, every beat included
    #[argh(switch)]
    json: bool,
    /// tempos are reported in [MIN, MAX), e.g. 75-175, the default
    #[argh(option, default = "stelly_mix::tempo::DJ_BPM", from_str_fn(bpm_range))]
    bpm_range: (f64, f64),
    /// audio files: MP3, FLAC, WAV, AIFF, Ogg, Opus, M4A
    #[argh(positional)]
    files: Vec<PathBuf>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "eval")]
/// Score beats, downbeats and tempo against annotated files.
struct Eval {
    /// list this many files with the worst beat F-measure
    #[argh(option, default = "0")]
    worst: usize,
    /// tempos are reported in [MIN, MAX), e.g. 75-175, the default
    #[argh(option, default = "stelly_mix::tempo::DJ_BPM", from_str_fn(bpm_range))]
    bpm_range: (f64, f64),
    /// directory of `.beats` annotations, Ballroom's format
    #[argh(positional)]
    annotations: PathBuf,
    /// directory holding the audio, searched recursively by file stem; each
    /// file's own directory names its group
    #[argh(positional)]
    audio: PathBuf,
}

fn main() -> Result<()> {
    let cli: Cli = argh::from_env();
    match cli.command {
        Command::Analyse(args) => {
            let mut failed = 0;
            let options = stelly_mix::Options {
                bpm_range: args.bpm_range,
            };
            for file in &args.files {
                if let Err(err) = analyse(file, args.json, &options) {
                    eprintln!("{}: {err:#}", file.display());
                    failed += 1;
                }
            }
            if failed > 0 {
                std::process::exit(1);
            }
        }
        Command::Eval(args) => evaluate(&args)?,
    }
    Ok(())
}

struct Scored {
    name: String,
    group: String,
    beat_f: f64,
    downbeat_f: f64,
    acc1: bool,
    acc2: bool,
}

fn evaluate(args: &Eval) -> Result<()> {
    let mut audio = HashMap::new();
    walk(&args.audio, &mut |path| {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            audio.insert(stem.to_string(), path.to_path_buf());
        }
    })?;
    let mut pairs = Vec::new();
    walk(&args.annotations, &mut |path| {
        if path.extension().is_some_and(|e| e == "beats") {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            if let Some(file) = audio.get(stem) {
                pairs.push((path.to_path_buf(), file.clone()));
            }
        }
    })?;
    pairs.sort();
    anyhow::ensure!(!pairs.is_empty(), "no annotation matched an audio file by name");
    eprintln!("{} annotated files", pairs.len());

    let started = Instant::now();
    let options = stelly_mix::Options {
        bpm_range: args.bpm_range,
    };
    let next = AtomicUsize::new(0);
    let scored = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((truth, file)) = pairs.get(i) else { break };
                match score(truth, file, &options) {
                    Ok(s) => scored.lock().unwrap().push(s),
                    Err(err) => eprintln!("{}: {err:#}", file.display()),
                }
            });
        }
    });
    let mut scored = scored.into_inner().unwrap();
    eprintln!("scored in {:.0}s", started.elapsed().as_secs_f64());

    let mut groups: BTreeMap<&str, Vec<&Scored>> = BTreeMap::new();
    for s in &scored {
        groups.entry(&s.group).or_default().push(s);
    }
    println!("{:<20} {:>5} {:>7} {:>7} {:>6} {:>6}", "", "files", "beat F", "down F", "Acc1", "Acc2");
    let row = |name: &str, rows: &[&Scored]| {
        let n = rows.len() as f64;
        let mean = |f: &dyn Fn(&Scored) -> f64| rows.iter().map(|s| f(s)).sum::<f64>() / n;
        println!(
            "{name:<20} {:>5} {:>7.3} {:>7.3} {:>6.3} {:>6.3}",
            rows.len(),
            mean(&|s| s.beat_f),
            mean(&|s| s.downbeat_f),
            mean(&|s| f64::from(u8::from(s.acc1))),
            mean(&|s| f64::from(u8::from(s.acc2)))
        );
    };
    for (group, rows) in &groups {
        row(group, rows);
    }
    row("all", &scored.iter().collect::<Vec<_>>());

    if args.worst > 0 {
        scored.sort_by(|a, b| a.beat_f.total_cmp(&b.beat_f));
        println!();
        for s in scored.iter().take(args.worst) {
            println!("{:.3}  {}/{}", s.beat_f, s.group, s.name);
        }
    }
    Ok(())
}

fn score(truth: &Path, file: &Path, options: &stelly_mix::Options) -> Result<Scored> {
    let truth = Truth::read(truth)?;
    let analysis = stelly_mix::analyse(&stelly_mix::decode::decode_file(file)?, options)?;
    let (acc1, acc2) = match (analysis.grid.as_ref(), truth.bpm()) {
        (Some(grid), Some(annotated)) => eval::tempo_accuracy(grid.bpm, annotated),
        _ => (false, false),
    };
    let group = file.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("");
    Ok(Scored {
        name: file.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string(),
        group: group.to_string(),
        beat_f: eval::f_measure(&analysis.beats, &truth.beats),
        downbeat_f: eval::f_measure(&analysis.downbeats, &truth.downbeats),
        acc1,
        acc2,
    })
}

/// Every file under `dir`.
fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, visit)?;
        } else {
            visit(&path);
        }
    }
    Ok(())
}

fn analyse(file: &Path, json: bool, options: &stelly_mix::Options) -> Result<()> {
    let started = Instant::now();
    let pcm = stelly_mix::decode::decode_file(file)?;
    let analysis = stelly_mix::analyse(&pcm, options)?;

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

/// "MIN-MAX", spanning at least an octave so every tempo has a place in it.
fn bpm_range(text: &str) -> Result<(f64, f64), String> {
    let (min, max) = text.split_once('-').ok_or("expected MIN-MAX, e.g. 75-175")?;
    let parse = |v: &str| v.trim().parse::<f64>().map_err(|e| format!("{v:?}: {e}"));
    let (min, max) = (parse(min)?, parse(max)?);
    if !(min > 0.0 && max >= 2.0 * min) {
        return Err(format!("{min}-{max} must span an octave, MAX at least twice MIN"));
    }
    Ok((min, max))
}

fn clock(seconds: f64) -> String {
    format!("{}:{:04.1}", (seconds / 60.0) as u64, seconds % 60.0)
}
