//! The command line: analyse files and print what a DJ would want to know.

use anyhow::{Context, Result};
use argh::FromArgs;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use stelly_audio::clap::AudioEncoder;
use stelly_mix::eval::{self, Truth};
use stelly_mix::library::Library;

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
    Index(Index),
    Next(Next),
    Mix(Mix),
    Set(Set),
    Find(Find),
    Eval(Eval),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "find")]
/// The library's tracks that sound most like a description.
struct Find {
    /// how many to list
    #[argh(option, default = "15")]
    top: usize,
    /// what to look for, e.g. "chill downtempo with soft pads"
    #[argh(positional)]
    description: String,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "set")]
/// Plan a set from a first track through the library, and render it.
struct Set {
    /// how long, in minutes
    #[argh(option, default = "30.0")]
    minutes: f64,
    /// where to write it
    #[argh(option, default = "PathBuf::from(\"set.wav\")")]
    out: PathBuf,
    /// only plan it, and print the tracklist
    #[argh(switch)]
    no_render: bool,
    /// largest tempo change from the first track's, in percent
    #[argh(option, default = "6.0")]
    stretch: f64,
    /// partial sets carried forward while planning
    #[argh(option, default = "8")]
    beam: usize,
    /// keep the set sounding like this, e.g. "chill downtempo"
    #[argh(option)]
    like: Option<String>,
    /// the track to start from, indexed first if it is not yet; with
    /// `--like`, the library's best match when left out
    #[argh(positional)]
    first: Option<PathBuf>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "mix")]
/// Render the best transition from one track into another to a WAV file.
struct Mix {
    /// where to write it
    #[argh(option, default = "PathBuf::from(\"transition.wav\")")]
    out: PathBuf,
    /// seconds of each track around the crossfade
    #[argh(option, default = "30.0")]
    lead: f64,
    /// largest tempo change, in percent
    #[argh(option, default = "6.0")]
    stretch: f64,
    /// let the pitch follow the tempo, as on a turntable, instead of keylock
    #[argh(switch)]
    turntable: bool,
    /// a plain equal-power crossfade instead of a bass swap
    #[argh(switch)]
    crossfade: bool,
    /// the track playing
    #[argh(positional)]
    from: PathBuf,
    /// the track to mix into
    #[argh(positional)]
    to: PathBuf,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "index")]
/// Analyse and embed every phrase of files and folders into the library.
struct Index {
    /// tempos are reported in [MIN, MAX), e.g. 88-176, the default
    #[argh(option, default = "stelly_mix::tempo::DJ_BPM", from_str_fn(bpm_range))]
    bpm_range: (f64, f64),
    /// audio files and folders, searched recursively
    #[argh(positional)]
    paths: Vec<PathBuf>,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "next")]
/// The best tracks of the library to mix into from this one, and where.
struct Next {
    /// how many to list
    #[argh(option, default = "10")]
    top: usize,
    /// largest tempo change, in percent
    #[argh(option, default = "6.0")]
    stretch: f64,
    /// the track playing now; indexed first if it is not yet
    #[argh(positional)]
    file: PathBuf,
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
        Command::Index(args) => index(&args)?,
        Command::Next(args) => next(&args)?,
        Command::Mix(args) => mix(&args)?,
        Command::Set(args) => set(&args)?,
        Command::Find(args) => find(&args)?,
        Command::Eval(args) => evaluate(&args)?,
    }
    Ok(())
}

const AUDIO: [&str; 8] = ["mp3", "flac", "wav", "aif", "aiff", "ogg", "opus", "m4a"];

fn index(args: &Index) -> Result<()> {
    let library = Library::open(Library::default_dir())?;
    let mut files = Vec::new();
    for path in &args.paths {
        if path.is_dir() {
            walk(path, &mut |p| files.push(p.to_path_buf()))?;
        } else {
            files.push(path.clone());
        }
    }
    files.retain(|f| {
        let audio = f.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO.contains(&e.to_ascii_lowercase().as_str()));
        audio && library.get(f).is_none()
    });
    files.sort();
    if files.is_empty() {
        eprintln!("everything is indexed already");
        return Ok(());
    }
    let models = stelly_mix::models::dir();
    stelly_mix::models::ensure(&models)?;

    let options = stelly_mix::Options {
        bpm_range: args.bpm_range,
    };
    // Each worker holds an ONNX session; a few already keep every core busy.
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(1, 8);
    let (next, done) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let started = Instant::now();
    std::thread::scope(|scope| -> Result<()> {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    let mut encoder = AudioEncoder::load(&models, 1)?;
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(file) = files.get(i) else { return Ok(()) };
                        match stelly_mix::library::index(file, &options, &mut encoder) {
                            Ok(track) => {
                                library.put(&track)?;
                                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                                eprintln!(
                                    "[{n}/{}] {} — {} phrases",
                                    files.len(),
                                    track.name(),
                                    track.segments.len()
                                );
                            }
                            Err(err) => eprintln!("{}: {err:#}", file.display()),
                        }
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("an indexing worker panicked")?;
        }
        Ok(())
    })?;
    eprintln!("indexed {} files in {:.0}s", done.into_inner(), started.elapsed().as_secs_f64());
    Ok(())
}

/// The library's entry for a file, indexing it first if need be.
fn indexed(library: &Library, file: &Path) -> Result<stelly_mix::library::Track> {
    if let Some(track) = library.get(file) {
        return Ok(track);
    }
    let models = stelly_mix::models::dir();
    stelly_mix::models::ensure(&models)?;
    let mut encoder = AudioEncoder::load(&models, 4)?;
    let track = stelly_mix::library::index(file, &stelly_mix::Options::default(), &mut encoder)?;
    library.put(&track)?;
    Ok(track)
}

fn mix(args: &Mix) -> Result<()> {
    let library = Library::open(Library::default_dir())?;
    let (playing, next) = (indexed(&library, &args.from)?, indexed(&library, &args.to)?);
    let options = stelly_mix::transitions::Options {
        max_stretch: args.stretch / 100.0,
        ..Default::default()
    };
    let transition = stelly_mix::transitions::from(&playing, std::slice::from_ref(&next), &options)
        .into_iter()
        .next()
        .with_context(|| {
            format!(
                "{:.2} and {:.2} BPM are more than {}% apart",
                playing.bpm().unwrap_or(0.0),
                next.bpm().unwrap_or(0.0),
                args.stretch
            )
        })?;
    let a = stelly_mix::decode::decode_stereo(&playing.path)?;
    let b = stelly_mix::decode::decode_stereo(&next.path)?;
    let settings = stelly_mix::render::Settings {
        keylock: !args.turntable,
        bass_swap: !args.crossfade,
    };
    let rendered = stelly_mix::render::render(&playing, &a, &next, &b, &transition, args.lead, &settings)?;
    let cue = &rendered.cues[0];
    let (fade_from, fade_to) = cue.transition.unwrap_or_default();
    stelly_mix::render::write_wav(&args.out, &rendered.audio)?;
    let (out, into) = (&playing.segments[transition.out], &next.segments[transition.into]);
    println!(
        "{}: {} at {} → {} at {}, {:+.1}% tempo, transition {} to {}{}",
        args.out.display(),
        playing.name(),
        clock(out.start),
        next.name(),
        clock(into.start),
        (transition.ratio - 1.0) * 100.0,
        clock(fade_from),
        clock(fade_to),
        cue.swap.map_or(String::new(), |s| format!(", basses swap at {}", clock(s)))
    );
    Ok(())
}

/// Every library track's likeness to `description`, 0..1 by rank.
fn liking(tracks: &[stelly_mix::library::Track], description: &str) -> Result<Vec<f64>> {
    let models = stelly_mix::models::dir();
    stelly_mix::models::ensure_text(&models)?;
    let mut encoder = stelly_mix::text::TextEncoder::load(&models)?;
    stelly_mix::text::scores(&mut encoder, tracks, description)
}

fn find(args: &Find) -> Result<()> {
    let tracks = Library::open(Library::default_dir())?.all()?;
    let like = liking(&tracks, &args.description)?;
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    order.sort_by(|&a, &b| like[b].total_cmp(&like[a]));
    for &i in order.iter().take(args.top) {
        let track = &tracks[i];
        println!(
            "{:.2}  {}  ({:.0} BPM, {})",
            like[i],
            track.name(),
            track.bpm().unwrap_or(0.0),
            track.analysis.key.as_ref().map_or("?", |k| k.camelot.as_str())
        );
    }
    Ok(())
}

fn set(args: &Set) -> Result<()> {
    let library = Library::open(Library::default_dir())?;
    if let Some(first) = &args.first {
        indexed(&library, first)?;
    }
    let tracks = library.all()?;
    let like = args.like.as_deref().map(|d| liking(&tracks, d)).transpose()?;
    let start = match (&args.first, &like) {
        (Some(first), _) => {
            let path = first.canonicalize()?;
            tracks.iter().position(|t| t.path == path).context("the first track is not in the library")?
        }
        (None, Some(like)) => (0..tracks.len())
            .filter(|&i| tracks[i].bpm().is_some() && tracks[i].analysis.grid.as_ref().is_some_and(|g| g.steady))
            .max_by(|&a, &b| like[a].total_cmp(&like[b]))
            .context("the library is empty")?,
        (None, None) => anyhow::bail!("give a first track, or --like to start from the best match"),
    };
    let options = stelly_mix::set::Options {
        seconds: args.minutes * 60.0,
        max_stretch: args.stretch / 100.0,
        beam: args.beam,
        like,
        ..Default::default()
    };
    let plan = stelly_mix::set::plan(&tracks, start, &options).context("the first track has no tempo")?;
    let chain: Vec<&stelly_mix::library::Track> = plan.tracks.iter().map(|&i| &tracks[i]).collect();
    eprintln!(
        "{} tracks, {} transitions averaging {:.2}, {} before the last comes in",
        chain.len(),
        plan.transitions.len(),
        plan.mean_score(),
        clock(plan.seconds)
    );

    let cues = if args.no_render {
        None
    } else {
        let settings = stelly_mix::render::Settings::default();
        let last_to = chain.last().map_or(0.0, |t| t.analysis.seconds);
        let started = Instant::now();
        let rendered = stelly_mix::render::render_chain(&chain, &plan.transitions, 0.0, last_to, &settings, |k| {
            stelly_mix::decode::decode_stereo(&chain[k].path)
        })?;
        stelly_mix::render::write_wav(&args.out, &rendered.audio)?;
        eprintln!(
            "{}: {} long, rendered in {:.0}s",
            args.out.display(),
            clock(rendered.audio.len() as f64 / rendered.audio.rate as f64),
            started.elapsed().as_secs_f64()
        );
        Some(rendered.cues)
    };

    let master = chain[0].bpm().unwrap_or(0.0);
    let key = |t: &stelly_mix::library::Track| t.analysis.key.as_ref().map_or("?".into(), |k| k.camelot.clone());
    for (k, track) in chain.iter().enumerate() {
        let at = cues.as_ref().map_or(String::new(), |c| format!("{}  ", clock(c[k].enters)));
        let bpm = track.bpm().unwrap_or(0.0);
        println!(
            "{at}{}  ({:.2} BPM{}, {})",
            track.name(),
            bpm,
            if k == 0 { String::new() } else { format!(" played at {master:.2}") },
            key(track)
        );
        if let Some(t) = plan.transitions.get(k) {
            let (out, into) = (&track.segments[t.out], &chain[k + 1].segments[t.into]);
            println!(
                "      ↳ out at {} into {}, score {:.2}, sound {:.2}",
                clock(out.start),
                clock(into.start),
                t.score,
                t.sound
            );
        }
    }
    Ok(())
}

fn next(args: &Next) -> Result<()> {
    let library = Library::open(Library::default_dir())?;
    let playing = indexed(&library, &args.file)?;
    let others = library.all()?;
    let options = stelly_mix::transitions::Options {
        max_stretch: args.stretch / 100.0,
        ..Default::default()
    };
    let transitions = stelly_mix::transitions::from(&playing, &others, &options);

    let key = |t: &stelly_mix::library::Track| t.analysis.key.as_ref().map_or("?".into(), |k| k.camelot.clone());
    println!(
        "{}: {:.2} BPM, {}, {} phrases; {} tracks within ±{}%",
        playing.name(),
        playing.bpm().unwrap_or(0.0),
        key(&playing),
        playing.segments.len(),
        transitions.len(),
        args.stretch
    );
    for t in transitions.iter().take(args.top) {
        let next = &others[t.to];
        let (out, into) = (&playing.segments[t.out], &next.segments[t.into]);
        println!();
        println!("{:.2}  {}", t.score, next.name());
        println!(
            "      out at {} → in at {}, {:+.1}% tempo, {} → {}{}, sound {:.2}{}",
            clock(out.start),
            clock(into.start),
            (t.ratio - 1.0) * 100.0,
            key(&playing),
            key(next),
            t.key_steps.map_or(String::new(), |s| format!(" ({s} step{})", if s == 1 { "" } else { "s" })),
            t.sound,
            t.energy.map_or(String::new(), |e| format!(", {e:+.1} LU")),
        );
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

/// "m:ss.s", rounded to the tenth first so 59.97s reads 1:00.0, not 0:60.0.
fn clock(seconds: f64) -> String {
    let tenths = (seconds * 10.0).round().max(0.0) as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

#[cfg(test)]
mod tests {
    #[test]
    fn clock_carries_into_the_minute() {
        assert_eq!(super::clock(59.97), "1:00.0");
        assert_eq!(super::clock(204.04), "3:24.0");
        assert_eq!(super::clock(5.0), "0:05.0");
    }
}
