//! Cue points in the shape Serato keeps them, which is also the shape Mixxx
//! and VirtualDJ read.
//!
//! Serato does not publish this, and none of the three programs would take a
//! Rekordbox XML: Mixxx wants a USB export built by Rekordbox itself, and
//! Serato keeps everything in the files. So the mix points go in the files,
//! in the one format all three already read, and one blob serves all of
//! them. The layout below is the reverse-engineered one the `serato-tags`
//! project documents and Mixxx's own importer is written against.
//!
//! What goes where:
//!
//! - FLAC: a `SERATO_MARKERS_V2` Vorbis comment, holding the blob wrapped in
//!   the same mime/filename/description envelope an ID3 `GEOB` frame carries
//!   in its own fields, then base64 without padding and a linefeed every 72
//!   characters, which is how Serato writes it.
//! - MP3: a `GEOB` frame named `Serato Markers2`, whose fields are that
//!   envelope, carrying the blob as it is — `GEOB` holds bytes, so there is
//!   nothing to base64 at that level.
//!
//! The blob is encoded twice over, which is the part worth knowing: two
//! version bytes, then the markers *themselves* base64'd and newline-broken
//! the same way, then a null, then padding. Reading the documentation as one
//! encoding produces bytes every reader rejects.

use base64::Engine;

/// The Vorbis comment a FLAC file keeps them in.
pub const FLAC_FIELD: &str = "SERATO_MARKERS_V2";
/// What the `GEOB` frame is called, and what names the blob inside the FLAC
/// envelope too.
pub const NAME: &str = "Serato Markers2";
/// The mime the envelope claims, in both containers.
pub const MIME: &str = "application/octet-stream";

/// Serato pads the blob out to this, so leave room however few cues there
/// are: a shorter one is read, but this is what its own writer produces.
const PADDED_TO: usize = 470;
/// Where the base64 breaks, as Serato writes it.
const WRAP: usize = 72;
/// Hot cue pads, 0..=7. More than this and the rest are dropped rather than
/// written to a pad that does not exist.
pub const PADS: usize = 8;

/// One cue point, in a track's own seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    /// Which pad, 0..PADS.
    pub index: u8,
    pub at: f64,
    /// What colour the pad lights.
    pub colour: [u8; 3],
    pub label: String,
}

/// Serato's green, for where a mix comes in.
pub const IN: [u8; 3] = [0x00, 0xCC, 0x00];
/// And its red, for where one goes out.
pub const OUT: [u8; 3] = [0xCC, 0x00, 0x00];

/// The markers themselves: two version bytes and one framed entry each.
/// This is what gets base64'd *inside* the blob.
fn entries(cues: &[Cue]) -> Vec<u8> {
    let mut out = vec![0x01, 0x01];
    for cue in cues.iter().filter(|cue| (cue.index as usize) < PADS).take(PADS) {
        let label = cue.label.as_bytes();
        let mut data = Vec::with_capacity(13 + label.len());
        data.push(0x00);
        data.push(cue.index);
        // Milliseconds, big-endian, from the front of the track.
        let ms = (cue.at.max(0.0) * 1000.0).round().min(u32::MAX as f64) as u32;
        data.extend_from_slice(&ms.to_be_bytes());
        data.push(0x00);
        data.extend_from_slice(&cue.colour);
        data.extend_from_slice(&[0x00, 0x00]);
        data.extend_from_slice(label);
        data.push(0x00);

        out.extend_from_slice(b"CUE\0");
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&data);
    }
    out
}

/// Base64 as Serato writes it: no padding, a linefeed every 72 characters.
fn wrapped(bytes: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes);
    encoded
        .as_bytes()
        .chunks(WRAP)
        .map(|line| std::str::from_utf8(line).expect("base64 is ASCII"))
        .collect::<Vec<&str>>()
        .join("\n")
}

/// The `Serato Markers2` blob: its own two version bytes, the markers
/// base64'd inside it, a null to end them, then padding.
pub fn markers2(cues: &[Cue]) -> Vec<u8> {
    let mut out = vec![0x01, 0x01];
    out.extend_from_slice(wrapped(&entries(cues)).as_bytes());
    out.push(0x00);
    out.resize(out.len().max(PADDED_TO), 0x00);
    out
}

/// The blob in the envelope both containers carry it in: the mime, an empty
/// filename, the name, then the bytes.
pub fn enveloped(cues: &[Cue]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MIME.as_bytes());
    out.push(0x00);
    // The filename, which Serato leaves empty.
    out.push(0x00);
    out.extend_from_slice(NAME.as_bytes());
    out.push(0x00);
    out.extend_from_slice(&markers2(cues));
    out
}

/// What a FLAC file's `SERATO_MARKERS_V2` comment holds.
pub fn flac_value(cues: &[Cue]) -> String {
    wrapped(&enveloped(cues))
}

/// Put `cues` into the file at `path`, where its kind keeps them.
///
/// Only the marker tag is touched; everything else in the file is left as it
/// was. A kind nothing is known about is passed over rather than rewritten,
/// since a half-tagged file is worse than an untagged one.
pub fn write(path: &std::path::Path, cues: &[Cue]) -> anyhow::Result<bool> {
    use anyhow::Context;
    use lofty::config::WriteOptions;
    use lofty::file::AudioFile;

    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("flac") => {
            let mut file = lofty::flac::FlacFile::read_from(
                &mut std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
                lofty::config::ParseOptions::new(),
            )
            .with_context(|| format!("reading {}", path.display()))?;
            if file.vorbis_comments().is_none() {
                file.set_vorbis_comments(Default::default());
            }
            file.vorbis_comments_mut()
                .context("a FLAC file takes Vorbis comments")?
                .insert(FLAC_FIELD.to_string(), flac_value(cues));
            let mut handle = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .with_context(|| format!("opening {} to write", path.display()))?;
            file.save_to(&mut handle, WriteOptions::default()).with_context(|| format!("tagging {}", path.display()))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cues() -> Vec<Cue> {
        vec![
            Cue { index: 0, at: 16.0, colour: IN, label: "mix in".into() },
            Cue { index: 1, at: 200.5, colour: OUT, label: "mix out".into() },
        ]
    }

    #[test]
    fn the_blob_starts_with_its_two_version_bytes_and_is_padded_out() {
        let blob = markers2(&cues());
        assert_eq!(&blob[..2], &[0x01, 0x01]);
        assert!(blob.len() >= PADDED_TO, "Serato's own writer pads to {PADDED_TO}");
        assert!(blob[blob.len() - 1] == 0x00);
        // The markers inside it are base64 again, ended by a null.
        let inner: String = blob[2..].iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
        assert!(inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '\n'));
    }

    #[test]
    fn a_cue_is_framed_by_name_then_length_then_its_fields() {
        let blob = entries(&[Cue { index: 3, at: 1.5, colour: [0x11, 0x22, 0x33], label: "x".into() }]);
        let at = 2;
        assert_eq!(&blob[at..at + 4], b"CUE\0");
        let len = u32::from_be_bytes(blob[at + 4..at + 8].try_into().unwrap()) as usize;
        // Twelve fixed bytes, the label, and the null that ends it.
        assert_eq!(len, 12 + 1 + 1);

        let data = &blob[at + 8..at + 8 + len];
        assert_eq!(data[0], 0x00, "reserved");
        assert_eq!(data[1], 3, "the pad it lands on");
        assert_eq!(u32::from_be_bytes(data[2..6].try_into().unwrap()), 1500, "milliseconds, big-endian");
        assert_eq!(data[6], 0x00, "reserved");
        assert_eq!(&data[7..10], &[0x11, 0x22, 0x33], "the pad's colour");
        assert_eq!(&data[10..12], &[0x00, 0x00], "reserved");
        assert_eq!(&data[12..], b"x\0", "the label, null-terminated");
    }

    #[test]
    fn the_envelope_names_the_blob_as_a_geob_frame_would() {
        let wrapped = enveloped(&cues());
        let head = b"application/octet-stream\0\0Serato Markers2\0";
        assert_eq!(&wrapped[..head.len()], head);
        assert_eq!(&wrapped[head.len()..head.len() + 2], &[0x01, 0x01], "the blob follows it");
    }

    #[test]
    fn a_flac_comment_is_unpadded_base64_broken_every_seventy_two() {
        let value = flac_value(&cues());
        assert!(!value.contains('='), "Serato writes it without padding");
        for line in value.split('\n') {
            assert!(line.len() <= WRAP, "{} is longer than {WRAP}", line.len());
        }
        assert!(value.split('\n').next().is_some_and(|first| first.len() == WRAP), "full lines but the last");
        // And it reads back to what went in.
        let joined: String = value.split('\n').collect();
        let decoded = base64::engine::general_purpose::STANDARD_NO_PAD.decode(joined).expect("valid base64");
        assert_eq!(decoded, enveloped(&cues()));
    }

    /// The one test that is not this module marking its own homework:
    /// `triseratops` is an independent reader of Serato's tags, written
    /// against the same reverse-engineered notes Mixxx's importer is. If it
    /// reads back what went in, the bytes are the right bytes.
    #[test]
    fn an_independent_reader_gets_back_the_cues_that_went_in() {
        use triseratops::tag::format::flac::FLACTag;

        let written = flac_value(&cues());
        let read = triseratops::tag::Markers2::parse_flac(written.as_bytes()).expect("a reader Mixxx agrees with");
        let found: Vec<&triseratops::tag::generic::Cue> = read.cues().collect();

        assert_eq!(found.len(), 2, "both cues came back");
        assert_eq!(found[0].index, 0);
        assert_eq!(found[0].position.millis, 16_000, "where the mix comes in");
        assert_eq!(found[0].label, "mix in");
        assert_eq!((found[0].color.red, found[0].color.green, found[0].color.blue), (0x00, 0xCC, 0x00));

        assert_eq!(found[1].index, 1);
        assert_eq!(found[1].position.millis, 200_500, "and where it goes out");
        assert_eq!(found[1].label, "mix out");
        assert_eq!((found[1].color.red, found[1].color.green, found[1].color.blue), (0xCC, 0x00, 0x00));
    }

    /// Into a real file and back out of it, with the tag read by lofty and
    /// the cues by the independent parser: the whole path a set takes to the
    /// decks, short of the decks.
    #[test]
    fn a_flac_file_carries_the_cues_and_keeps_what_it_had() {
        use lofty::file::AudioFile;
        use triseratops::tag::format::flac::FLACTag;

        // A minimal FLAC: a STREAMINFO block and no audio. Enough to tag.
        let mut flac = Vec::from(*b"fLaC");
        let streaminfo = [
            0x80u8, 0x00, 0x00, 0x22, // last block, STREAMINFO, 34 bytes
            0x10, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x0a, 0xc4, 0x42, 0xf0, 0x00, 0x00, 0x00, 0x00,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        flac.extend_from_slice(&streaminfo);
        let dir = std::env::temp_dir().join(format!("stelly-mix-serato-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.flac");
        std::fs::write(&path, &flac).unwrap();

        // Something already in the file, to be sure tagging does not wipe it.
        {
            let mut file =
                lofty::flac::FlacFile::read_from(&mut std::fs::File::open(&path).unwrap(), lofty::config::ParseOptions::new()).unwrap();
            file.set_vorbis_comments(Default::default());
            file.vorbis_comments_mut().unwrap().insert("TITLE".into(), "A Song".into());
            let mut handle = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
            file.save_to(&mut handle, lofty::config::WriteOptions::default()).unwrap();
        }

        assert!(write(&path, &cues()).unwrap(), "a FLAC is a kind it knows");

        let file =
            lofty::flac::FlacFile::read_from(&mut std::fs::File::open(&path).unwrap(), lofty::config::ParseOptions::new()).unwrap();
        let comments = file.vorbis_comments().expect("comments");
        assert_eq!(comments.get("TITLE"), Some("A Song"), "what was there is still there");

        let value = comments.get(FLAC_FIELD).expect("the markers are in the file");
        let read = triseratops::tag::Markers2::parse_flac(value.as_bytes()).expect("Serato and Mixxx would read this");
        let found: Vec<&triseratops::tag::generic::Cue> = read.cues().collect();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].position.millis, 16_000);
        assert_eq!(found[1].label, "mix out");

        // A kind it does not know is left alone rather than half-written.
        let other = dir.join("a.wav");
        std::fs::write(&other, b"RIFF").unwrap();
        assert!(!write(&other, &cues()).unwrap());
        assert_eq!(std::fs::read(&other).unwrap(), b"RIFF");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_pads_that_exist_are_written() {
        let many: Vec<Cue> = (0..12).map(|i| Cue { index: i, at: i as f64, colour: IN, label: String::new() }).collect();
        let blob = entries(&many);
        assert_eq!(blob.windows(4).filter(|w| *w == b"CUE\0").count(), PADS, "eight pads, not twelve");
    }

    #[test]
    fn a_cue_before_the_start_does_not_wrap_round() {
        let blob = entries(&[Cue { index: 0, at: -5.0, colour: IN, label: String::new() }]);
        let data = &blob[10..];
        assert_eq!(u32::from_be_bytes(data[2..6].try_into().unwrap()), 0);
    }
}
