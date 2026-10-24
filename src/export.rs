//! Handing a set to the decks.
//!
//! stelly-mix plans; the set is played somewhere else. What matters is that
//! the planning survives the crossing: not just the order of the tracks, but
//! where each one comes in and goes out, which is the part that took the
//! analysis to work out and the part a DJ would otherwise find again by ear.
//!
//! So an export is a folder holding the tracks themselves, a playlist, and —
//! where the program on the other side can read them — the mix points as
//! cues already placed, with the grid and the key beside them. A rendered
//! reference mix goes in too, to listen back to while rehearsing it.
//!
//! The cue times are in each track's own seconds, taken from its segments,
//! which is what the other program wants: it is reading the same file.

use crate::library::Track;
use crate::render;
use crate::set::Plan;
use crate::sets::Set;
use crate::source::Sources;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// What the set is being handed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Rekordbox XML: the collection with grid and memory cues, and a
    /// playlist of it.
    Rekordbox,
    /// Traktor's collection.nml, the same in its own shape.
    Traktor,
    /// Engine reads the playlist, and its conversion tool reads Rekordbox's
    /// XML for the cues, so it gets both.
    Engine,
    /// Serato keeps its cues inside the files; they are written there, so
    /// they arrive placed.
    Serato,
    /// Mixxx will not read a Rekordbox XML — it wants a USB export built by
    /// Rekordbox itself — but it does read Serato's tags, so it gets those.
    Mixxx,
    /// No program in particular: the playlist, a cue sheet and the tracks.
    Plain,
}

impl Target {
    pub const ALL: [Target; 6] =
        [Target::Rekordbox, Target::Traktor, Target::Engine, Target::Serato, Target::Mixxx, Target::Plain];

    pub fn as_str(self) -> &'static str {
        match self {
            Target::Rekordbox => "rekordbox",
            Target::Traktor => "traktor",
            Target::Engine => "engine",
            Target::Serato => "serato",
            Target::Mixxx => "mixxx",
            Target::Plain => "plain",
        }
    }

    /// For people.
    pub fn title(self) -> &'static str {
        match self {
            Target::Rekordbox => "Rekordbox",
            Target::Traktor => "Traktor",
            Target::Engine => "Engine DJ",
            Target::Serato => "Serato",
            Target::Mixxx => "Mixxx",
            Target::Plain => "Anything",
        }
    }

    /// What it will not manage, in a sentence, or nothing.
    ///
    /// Said plainly rather than discovered on the night: Serato keeps cues in
    /// a tag inside each file, in a shape of its own that is not written
    /// here, so its tracks arrive in order but without the mix points.
    pub fn caveat(self) -> Option<&'static str> {
        match self {
            Target::Engine => Some("Engine reads the playlist by itself; the mix points come across through its Rekordbox conversion."),
            Target::Mixxx => Some("Mixxx will not read a Rekordbox XML, so the mix points reach it through Serato's tags in the files, which it reads natively."),
            _ => None,
        }
    }

    /// Whether the mix points cross as cues the other program will show.
    /// All of them do now: the ones with a library format of their own get
    /// it, and the rest read the tags written into the files.
    pub fn carries_cues(self) -> bool {
        true
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Target {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        Target::ALL
            .into_iter()
            .find(|target| target.as_str() == text.trim().to_lowercase())
            .ok_or_else(|| format!("{text:?} is not an export: rekordbox, traktor, engine, serato or plain"))
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub target: Target,
    /// Copy each track's audio into the folder. Without it the playlist
    /// points at the cache, which is not somewhere to hand anyone.
    pub tracks: bool,
    /// Render the whole set to listen back to.
    pub reference: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { target: Target::Rekordbox, tracks: true, reference: true }
    }
}

/// What was written.
#[derive(Debug, Clone)]
pub struct Exported {
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
    pub bytes: u64,
    /// The set's tracks the library no longer holds, left out of the folder.
    pub missing: Vec<String>,
}

/// One track as the export writes it.
#[derive(Debug, Clone)]
pub struct Entry {
    /// 1-based, as the file names are numbered.
    pub position: usize,
    /// Where its audio ended up, absolute.
    pub file: PathBuf,
    /// Its name in the folder, which is what the playlist points at.
    pub name: String,
    pub artist: String,
    pub title: String,
    pub seconds: f64,
    pub bpm: Option<f64>,
    /// The first beat of the grid, seconds: where a beat grid is anchored.
    pub first_beat: Option<f64>,
    /// Camelot, "8A".
    pub camelot: Option<String>,
    /// The same key written as a musician would, "Am".
    pub tonality: Option<String>,
    /// Where the track before mixes into this one, in this track's seconds.
    pub mix_in: Option<f64>,
    /// Where this one mixes out, in this track's seconds.
    pub mix_out: Option<f64>,
    /// Where it comes in, in the reference mix's seconds.
    pub enters: Option<f64>,
}

/// Write `set` into a folder of its own under `into`.
///
/// Whatever is missing from the library is left out and reported rather than
/// refused: a set is worth handing over with a hole in it.
pub fn write(set: &Set, library: &[Track], sources: &Sources, into: &Path, options: &Options, report: &dyn Fn(&str)) -> Result<Exported> {
    let resolved = set.resolve(library);
    let plan = resolved.plan;
    if plan.tracks.is_empty() {
        bail!("none of {}'s tracks are in the library any more", set.name);
    }
    let chain: Vec<&Track> = plan.tracks.iter().map(|&at| &library[at]).collect();

    let dir = into.join(safe(&set.name));
    std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
    // Absolute from here on: a playlist written with `--into .` is no use to
    // the program that opens it from somewhere else.
    let dir = dir.canonicalize().unwrap_or(dir);
    let mut files = Vec::new();

    // Where each track's audio is now, which is also what says what kind of
    // file it is. Fetched when it is being copied; otherwise only if it
    // happens to be here already.
    let audio: Vec<Option<PathBuf>> = chain
        .iter()
        .map(|track| {
            if options.tracks {
                report(&format!("fetching {}", track.name()));
                sources.audio(track).map(Some).with_context(|| format!("fetching {}", track.name()))
            } else {
                Ok(sources.cached(track))
            }
        })
        .collect::<Result<_>>()?;

    let mut entries = entries(&chain, &plan, &dir, &audio)?;
    if options.tracks {
        for (entry, from) in entries.iter().zip(&audio) {
            let Some(from) = from else { continue };
            report(&format!("copying {}", entry.name));
            std::fs::copy(from, &entry.file).with_context(|| format!("copying {} into {}", from.display(), entry.file.display()))?;
            // Into the copy, never the cache: the folder is what is handed
            // over, and the original is nobody's to rewrite.
            crate::serato::write(&entry.file, &marks_as_cues(entry))?;
            files.push(entry.file.clone());
        }
    }

    if options.reference {
        let mix = dir.join(format!("{}.wav", safe(&set.name)));
        report("rendering the set");
        let cues = reference(&chain, &plan, sources, &mix)?;
        for (entry, cue) in entries.iter_mut().zip(&cues) {
            entry.enters = Some(cue.enters);
        }
        files.push(mix);
    }

    let playlist = dir.join(format!("{}.m3u8", safe(&set.name)));
    std::fs::write(&playlist, m3u8(&entries)).with_context(|| format!("writing {}", playlist.display()))?;
    files.push(playlist);

    if options.reference {
        let sheet = dir.join(format!("{}.cue", safe(&set.name)));
        std::fs::write(&sheet, cue_sheet(set, &entries)).with_context(|| format!("writing {}", sheet.display()))?;
        files.push(sheet);
    }

    match options.target {
        Target::Rekordbox | Target::Engine => {
            let xml = dir.join("rekordbox.xml");
            std::fs::write(&xml, rekordbox(set, &entries)).with_context(|| format!("writing {}", xml.display()))?;
            files.push(xml);
        }
        Target::Traktor => {
            let nml = dir.join("collection.nml");
            std::fs::write(&nml, traktor(set, &entries)).with_context(|| format!("writing {}", nml.display()))?;
            files.push(nml);
        }
        Target::Serato | Target::Mixxx | Target::Plain => {}
    }

    let bytes = files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
    Ok(Exported { dir, files, bytes, missing: resolved.missing })
}

/// What each track is, and where it mixes, before anything is written.
///
/// A track's `file` is where it goes in the folder, whether or not the audio
/// is copied in: the folder is what is handed over, and a path to this
/// machine's cache would mean nothing on the other side.
fn entries(chain: &[&Track], plan: &Plan, dir: &Path, audio: &[Option<PathBuf>]) -> Result<Vec<Entry>> {
    chain
        .iter()
        .enumerate()
        .map(|(k, track)| {
            let (artist, title) = match &track.origin {
                Some(origin) => (origin.item.artist.clone(), origin.item.title.clone()),
                None => (String::new(), track.name()),
            };
            // From the file itself; a track from a source is named after
            // the recording, and carries no extension of its own.
            let extension = audio
                .get(k)
                .and_then(|path| path.as_ref())
                .and_then(|path| path.extension())
                .or_else(|| track.path.extension())
                .and_then(|e| e.to_str())
                .unwrap_or("audio")
                .to_string();
            let name = format!("{:02} - {}.{}", k + 1, safe(&format!("{artist} - {title}").replace(" - ", " - ")), extension);
            let grid = track.analysis.grid.as_ref();
            Ok(Entry {
                position: k + 1,
                file: dir.join(&name),
                name,
                artist,
                title,
                seconds: track.analysis.seconds,
                bpm: grid.map(|g| g.bpm),
                first_beat: grid.map(|g| g.first_beat),
                camelot: track.analysis.key.as_ref().map(|key| key.camelot.clone()),
                tonality: track.analysis.key.as_ref().map(tonality),
                // The track before mixes into this one at this one's `into`.
                mix_in: plan
                    .transitions
                    .get(k.wrapping_sub(1))
                    .filter(|_| k > 0)
                    .and_then(|t| track.segments.get(t.into))
                    .map(|segment| segment.start),
                mix_out: plan.transitions.get(k).and_then(|t| track.segments.get(t.out)).map(|segment| segment.start),
                enters: None,
            })
        })
        .collect()
}

/// Render the whole set to `mix`, and say where each track comes in.
fn reference(chain: &[&Track], plan: &Plan, sources: &Sources, mix: &Path) -> Result<Vec<render::Cue>> {
    let settings = render::Settings::default();
    let last_to = chain.last().map_or(0.0, |track| track.analysis.seconds);
    let rendered = render::render_chain(chain, &plan.transitions, 0.0, last_to, &settings, |k| {
        crate::decode::decode_file_stereo(&sources.audio(chain[k])?)
    })?;
    render::write_wav(mix, &rendered.audio)?;
    Ok(rendered.cues)
}

/// The key as a musician writes it, which is what Rekordbox and Traktor
/// show: "Am", "C".
fn tonality(key: &crate::key::Key) -> String {
    if key.scale.starts_with('m') {
        format!("{}m", key.tonic)
    } else {
        key.tonic.clone()
    }
}

fn m3u8(entries: &[Entry]) -> String {
    let mut out = String::from("#EXTM3U\n");
    for entry in entries {
        out.push_str(&format!("#EXTINF:{},{} - {}\n", entry.seconds.round() as i64, entry.artist, entry.title));
        out.push_str(&entry.name);
        out.push('\n');
    }
    out
}

/// A cue sheet over the reference mix, so a player shows the tracklist and
/// can skip between them. Its times are the mix's, not each track's.
fn cue_sheet(set: &Set, entries: &[Entry]) -> String {
    let mut out = String::new();
    out.push_str(&format!("TITLE {}\n", quoted(&set.name)));
    out.push_str("PERFORMER \"stelly-mix\"\n");
    out.push_str(&format!("FILE {} WAVE\n", quoted(&format!("{}.wav", safe(&set.name)))));
    for entry in entries {
        out.push_str(&format!("  TRACK {:02} AUDIO\n", entry.position));
        out.push_str(&format!("    TITLE {}\n", quoted(&entry.title)));
        out.push_str(&format!("    PERFORMER {}\n", quoted(&entry.artist)));
        out.push_str(&format!("    INDEX 01 {}\n", frames(entry.enters.unwrap_or(0.0))));
    }
    out
}

/// `MM:SS:FF`, the 75-frames-a-second a cue sheet counts in. Counted in
/// frames throughout, so a rounded one carries into the second rather than
/// landing on a 75th frame, which does not exist.
fn frames(seconds: f64) -> String {
    let total = (seconds.max(0.0) * 75.0).round() as u64;
    format!("{:02}:{:02}:{:02}", total / 75 / 60, total / 75 % 60, total % 75)
}

/// Rekordbox's XML: the tracks with their grid and the mix points as memory
/// cues, then a playlist naming them in order.
fn rekordbox(set: &Set, entries: &[Entry]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<DJ_PLAYLISTS Version=\"1.0.0\">\n");
    out.push_str(&format!("  <PRODUCT Name=\"stelly-mix\" Version=\"{}\" Company=\"stelly-mix\"/>\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("  <COLLECTION Entries=\"{}\">\n", entries.len()));
    for entry in entries {
        out.push_str(&format!(
            "    <TRACK TrackID=\"{}\" Name=\"{}\" Artist=\"{}\" TotalTime=\"{}\"{}{} Location=\"{}\">\n",
            entry.position,
            escape(&entry.title),
            escape(&entry.artist),
            entry.seconds.round() as i64,
            entry.bpm.map(|bpm| format!(" AverageBpm=\"{bpm:.2}\"")).unwrap_or_default(),
            entry.tonality.as_deref().map(|key| format!(" Tonality=\"{}\"", escape(key))).unwrap_or_default(),
            file_url(&entry.file),
        ));
        if let (Some(bpm), Some(first)) = (entry.bpm, entry.first_beat) {
            out.push_str(&format!("      <TEMPO Inizio=\"{first:.3}\" Bpm=\"{bpm:.2}\" Metro=\"4/4\" Battito=\"1\"/>\n"));
        }
        // Num="-1" is a memory cue: shown on the waveform, not bound to a
        // pad, which is what a planned mix point should be.
        for (name, at) in marks(entry) {
            out.push_str(&format!("      <POSITION_MARK Name=\"{name}\" Type=\"0\" Start=\"{at:.3}\" Num=\"-1\"/>\n"));
        }
        out.push_str("    </TRACK>\n");
    }
    out.push_str("  </COLLECTION>\n  <PLAYLISTS>\n    <NODE Type=\"0\" Name=\"ROOT\" Count=\"1\">\n");
    out.push_str(&format!("      <NODE Name=\"{}\" Type=\"1\" KeyType=\"0\" Entries=\"{}\">\n", escape(&set.name), entries.len()));
    for entry in entries {
        out.push_str(&format!("        <TRACK Key=\"{}\"/>\n", entry.position));
    }
    out.push_str("      </NODE>\n    </NODE>\n  </PLAYLISTS>\n</DJ_PLAYLISTS>\n");
    out
}

/// Traktor's collection, the same again in its own shape. Its cue times are
/// milliseconds, and its paths break the directory at every separator.
fn traktor(set: &Set, entries: &[Entry]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n");
    out.push_str("<NML VERSION=\"19\">\n");
    out.push_str("  <HEAD COMPANY=\"stelly-mix\" PROGRAM=\"stelly-mix\"/>\n");
    out.push_str(&format!("  <COLLECTION ENTRIES=\"{}\">\n", entries.len()));
    for entry in entries {
        out.push_str(&format!("    <ENTRY TITLE=\"{}\" ARTIST=\"{}\">\n", escape(&entry.title), escape(&entry.artist)));
        let parent = entry.file.parent().map(traktor_dir).unwrap_or_else(|| "/:".into());
        let file = entry.file.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        out.push_str(&format!("      <LOCATION DIR=\"{}\" FILE=\"{}\" VOLUME=\"\"/>\n", escape(&parent), escape(file)));
        out.push_str(&format!(
            "      <INFO PLAYTIME=\"{}\"{}/>\n",
            entry.seconds.round() as i64,
            entry.camelot.as_deref().map(|key| format!(" KEY=\"{}\"", escape(key))).unwrap_or_default()
        ));
        if let Some(bpm) = entry.bpm {
            out.push_str(&format!("      <TEMPO BPM=\"{bpm:.6}\" BPM_QUALITY=\"100.000000\"/>\n"));
        }
        for (order, (name, at)) in marks(entry).into_iter().enumerate() {
            out.push_str(&format!(
                "      <CUE_V2 NAME=\"{name}\" DISPLAY_ORDER=\"{order}\" TYPE=\"0\" START=\"{:.6}\" LEN=\"0\" REPEATS=\"-1\" HOTCUE=\"{order}\"/>\n",
                at * 1000.0
            ));
        }
        out.push_str("    </ENTRY>\n");
    }
    out.push_str("  </COLLECTION>\n  <PLAYLISTS>\n");
    out.push_str("    <NODE TYPE=\"FOLDER\" NAME=\"$ROOT\">\n      <SUBNODES COUNT=\"1\">\n");
    out.push_str(&format!("        <NODE TYPE=\"PLAYLIST\" NAME=\"{}\">\n", escape(&set.name)));
    out.push_str(&format!("          <PLAYLIST ENTRIES=\"{}\" TYPE=\"LIST\">\n", entries.len()));
    for entry in entries {
        let key = format!("{}/:{}", entry.file.parent().map(traktor_dir).unwrap_or_default(), entry.file.file_name().and_then(|n| n.to_str()).unwrap_or_default());
        out.push_str(&format!("            <ENTRY><PRIMARYKEY TYPE=\"TRACK\" KEY=\"{}\"/></ENTRY>\n", escape(&key)));
    }
    out.push_str("          </PLAYLIST>\n        </NODE>\n      </SUBNODES>\n    </NODE>\n  </PLAYLISTS>\n</NML>\n");
    out
}

/// The same mix points as Serato's hot cues, which Serato, Mixxx and
/// VirtualDJ all read out of the file itself.
fn marks_as_cues(entry: &Entry) -> Vec<crate::serato::Cue> {
    marks(entry)
        .into_iter()
        .enumerate()
        .map(|(pad, (label, at))| crate::serato::Cue {
            index: pad as u8,
            at,
            colour: if label == "mix in" { crate::serato::IN } else { crate::serato::OUT },
            label: label.to_string(),
        })
        .collect()
}

/// The mix points a track carries, in the order they happen.
fn marks(entry: &Entry) -> Vec<(&'static str, f64)> {
    let mut marks = Vec::new();
    if let Some(at) = entry.mix_in {
        marks.push(("mix in", at));
    }
    if let Some(at) = entry.mix_out {
        marks.push(("mix out", at));
    }
    marks
}

/// Traktor writes a directory as `/:path/:to/:`.
fn traktor_dir(path: &Path) -> String {
    let mut out = String::new();
    for part in path.components() {
        let part = part.as_os_str().to_string_lossy();
        if part == "/" {
            continue;
        }
        out.push_str("/:");
        out.push_str(&part);
    }
    out.push_str("/:");
    out
}

/// `file://localhost/…`, percent-encoded, as Rekordbox writes locations.
fn file_url(path: &Path) -> String {
    let mut out = String::from("file://localhost");
    let path = path.to_string_lossy();
    if !path.starts_with('/') {
        out.push('/');
    }
    for byte in path.bytes() {
        match byte {
            b'/' => out.push('/'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A name safe to put on a disk anyone might read it on: no separators, none
/// of what Windows refuses, no trailing dots or spaces.
fn safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "set".into()
    } else {
        trimmed.chars().take(96).collect()
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// A cue sheet's quoting: it has none, so a quote comes out.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            position: 1,
            file: PathBuf::from("/music/sets/Friday/01 - Lun - A Song.flac"),
            name: "01 - Lun - A Song.flac".into(),
            artist: "Lun".into(),
            title: "A Song".into(),
            seconds: 240.0,
            bpm: Some(128.0),
            first_beat: Some(0.125),
            camelot: Some("8A".into()),
            tonality: Some("Am".into()),
            mix_in: Some(16.0),
            mix_out: Some(200.5),
            enters: Some(0.0),
        }
    }

    #[test]
    fn targets_read_and_write_by_name() {
        for target in Target::ALL {
            assert_eq!(target.as_str().parse::<Target>(), Ok(target));
        }
        assert!("rekordbox ".parse::<Target>().is_ok());
        assert!("ableton".parse::<Target>().is_err());
    }

    #[test]
    fn every_target_carries_its_cues_one_way_or_another() {
        for target in Target::ALL {
            assert!(target.carries_cues(), "{target} should carry the mix points");
        }
        // The two that get there by a route worth explaining say so.
        assert!(Target::Mixxx.caveat().is_some(), "Mixxx reads Serato's tags, not a Rekordbox XML");
        assert!(Target::Engine.caveat().is_some());
        assert!(Target::Serato.caveat().is_none(), "Serato reads the tags straight off");
    }

    #[test]
    fn the_mix_points_become_hot_cues_on_their_own_pads() {
        let cues = marks_as_cues(&entry());
        assert_eq!(cues.len(), 2);
        assert_eq!((cues[0].index, cues[0].at, cues[0].label.as_str()), (0, 16.0, "mix in"));
        assert_eq!(cues[0].colour, crate::serato::IN);
        assert_eq!((cues[1].index, cues[1].at, cues[1].label.as_str()), (1, 200.5, "mix out"));
        assert_eq!(cues[1].colour, crate::serato::OUT);

        // A track at the end of a set goes out nowhere, so it has one.
        let last = Entry { mix_out: None, ..entry() };
        assert_eq!(marks_as_cues(&last).len(), 1);
    }

    #[test]
    fn rekordbox_carries_the_grid_and_both_mix_points() {
        let xml = rekordbox(&set("Friday"), &[entry()]);
        assert!(xml.contains("<TEMPO Inizio=\"0.125\" Bpm=\"128.00\" Metro=\"4/4\" Battito=\"1\"/>"), "{xml}");
        assert!(xml.contains("Name=\"mix in\" Type=\"0\" Start=\"16.000\" Num=\"-1\""), "{xml}");
        assert!(xml.contains("Name=\"mix out\" Type=\"0\" Start=\"200.500\" Num=\"-1\""), "{xml}");
        assert!(xml.contains("Tonality=\"Am\"") && xml.contains("AverageBpm=\"128.00\""), "{xml}");
        assert!(xml.contains("<TRACK Key=\"1\"/>"), "the playlist names it: {xml}");
    }

    #[test]
    fn traktor_counts_its_cues_in_milliseconds() {
        let nml = traktor(&set("Friday"), &[entry()]);
        assert!(nml.contains("START=\"16000.000000\""), "{nml}");
        assert!(nml.contains("START=\"200500.000000\""), "{nml}");
        assert!(nml.contains("KEY=\"8A\"") && nml.contains("BPM=\"128.000000\""), "{nml}");
        assert!(nml.contains("DIR=\"/:music/:sets/:Friday/:\""), "{nml}");
    }

    #[test]
    fn a_track_with_no_grid_or_key_still_writes() {
        let bare = Entry { bpm: None, first_beat: None, camelot: None, tonality: None, mix_in: None, mix_out: None, ..entry() };
        let xml = rekordbox(&set("Friday"), std::slice::from_ref(&bare));
        assert!(!xml.contains("<TEMPO") && !xml.contains("POSITION_MARK") && !xml.contains("AverageBpm"), "{xml}");
        let nml = traktor(&set("Friday"), std::slice::from_ref(&bare));
        // Not a bare `KEY=` search: the playlist's PRIMARYKEY carries one too.
        assert!(nml.contains("<INFO PLAYTIME=\"240\"/>"), "no key on the info line: {nml}");
        assert!(!nml.contains("<TEMPO") && !nml.contains("CUE_V2"), "{nml}");
    }

    #[test]
    fn names_and_text_cannot_break_out_of_the_files_they_go_in() {
        assert_eq!(safe("Friday / Saturday"), "Friday - Saturday");
        assert_eq!(safe("../../etc/passwd"), "..-..-etc-passwd", "no separator survives, so it cannot climb out of the folder");
        assert_eq!(safe("  trailing.  "), "trailing");
        assert_eq!(safe(""), "set");
        assert_eq!(escape("Bob & \"Alice\" <3"), "Bob &amp; &quot;Alice&quot; &lt;3");
        assert_eq!(quoted("a \"name\""), "\"a 'name'\"");
        assert!(file_url(Path::new("/music/a b&c.flac")).ends_with("/music/a%20b%26c.flac"));
    }

    #[test]
    fn a_cue_sheet_counts_in_seventy_fifths() {
        assert_eq!(frames(0.0), "00:00:00");
        assert_eq!(frames(61.5), "01:01:38");
        assert_eq!(frames(-1.0), "00:00:00");
        // 74 is the last frame of a second, never 75.
        assert_eq!(frames(1.9999), "00:02:00");
        assert_eq!(frames(1.98), "00:01:74");
    }

    #[test]
    fn the_playlist_and_the_sheet_name_what_is_in_the_folder() {
        let entries = [entry()];
        let m3u = m3u8(&entries);
        assert!(m3u.starts_with("#EXTM3U\n"));
        assert!(m3u.contains("#EXTINF:240,Lun - A Song\n01 - Lun - A Song.flac\n"), "{m3u}");
        let sheet = cue_sheet(&set("Friday"), &entries);
        assert!(sheet.contains("FILE \"Friday.wav\" WAVE"), "{sheet}");
        assert!(sheet.contains("    INDEX 01 00:00:00"), "{sheet}");
    }

    fn set(name: &str) -> Set {
        Set {
            id: "friday-0000".into(),
            name: name.into(),
            version: crate::sets::VERSION,
            created: 0,
            modified: 0,
            tracks: vec!["stelly:1".into()],
            steps: Vec::new(),
            seconds: 240.0,
            mean_score: 0.5,
            planned: crate::sets::Planned::default(),
        }
    }
}
