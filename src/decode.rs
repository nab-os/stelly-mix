//! Files in: whatever `stelly-audio` decodes, plus Opus.
//!
//! symphonia demuxes Ogg Opus but has no Opus decoder, and Opus is what most
//! audio downloaded from the web arrives as. So the packets symphonia reads
//! go through `rusty-opus`, pure Rust like the rest. The encoder's
//! pre-skip comes as the track's delay and the final padding as the last
//! packet's end trim; dropping both matters here, where a beat 6.5ms late is
//! a grid 6.5ms off.

use anyhow::{bail, Context, Result};
use std::path::Path;
use stelly_audio::decode::Pcm;

/// Opus always decodes at 48kHz; its timestamps count at that rate too.
const OPUS_RATE: u32 = 48_000;
/// The longest Opus packet, 120ms, in samples.
const MAX_PACKET: usize = 5_760;

/// Decode a whole file to mono.
pub fn decode_file(path: &Path) -> Result<Pcm> {
    let opus = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("opus"));
    if opus {
        return decode_opus(path);
    }
    match stelly_audio::decode::decode_file(path) {
        Ok(pcm) => Ok(pcm),
        // An .ogg can hold Opus too.
        Err(err) => decode_opus(path).map_err(|_| err),
    }
}

fn decode_opus(path: &Path) -> Result<Pcm> {
    use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let mut format = symphonia::default::get_probe()
        .probe(&hint, stream, FormatOptions::default(), MetadataOptions::default())
        .with_context(|| format!("{} is not an Ogg stream", path.display()))?;

    let track = format.default_track(TrackType::Audio).context("no audio track")?;
    let is_opus = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .is_some_and(|p| p.codec == CODEC_ID_OPUS);
    if !is_opus {
        bail!("{} holds no Opus", path.display());
    }
    let track_id = track.id;
    let mut pre_skip = track.delay.unwrap_or(0) as usize;

    // Mono straight from the decoder, whose downmix is the plain mean of the
    // channels, as every other format gets.
    let mut decoder = rusty_opus::OpusDecoder::new(OPUS_RATE as i32, 1).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut frame = vec![0f32; MAX_PACKET];
    let mut samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(Error::IoError(_)) => break,
            Err(err) => return Err(err).context("demuxing"),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet.data, MAX_PACKET, &mut frame) {
            Ok(n) => n,
            // A corrupt packet: conceal it rather than shift everything after.
            Err(_) => decoder.decode(&[], MAX_PACKET, &mut frame).unwrap_or(0),
        };
        let end = decoded.saturating_sub(packet.trim_end.get() as usize);
        let trim = (packet.trim_start.get() as usize).min(end);
        // The pre-skip can outlast the first packet.
        let skip = pre_skip.min(end - trim);
        pre_skip -= skip;
        samples.extend_from_slice(&frame[trim + skip..end]);
    }

    if samples.is_empty() {
        bail!("decoded no audio from {}", path.display());
    }
    Ok(Pcm {
        samples,
        rate: OPUS_RATE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clicks at 0.5s, 1.0s and 1.5s, encoded by libopus 1.5 through ffmpeg:
    /// `ffmpeg -i clicks.wav -c:a libopus -b:a 64k clicks.opus`. Its pre-skip
    /// is the usual 312 samples; left in, every click would land 6.5ms late.
    #[test]
    fn opus_clicks_land_where_they_were_put() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/clicks.opus");
        let pcm = decode_file(&path).unwrap();
        assert_eq!(pcm.rate, OPUS_RATE);
        assert!((pcm.seconds() - 2.0).abs() < 0.001, "{}s", pcm.seconds());

        for at in [0.5, 1.0, 1.5] {
            let from = ((at - 0.1) * OPUS_RATE as f64) as usize;
            let window = &pcm.samples[from..from + (0.2 * OPUS_RATE as f64) as usize];
            let peak = window.iter().map(|v| v.abs()).fold(0f32, f32::max);
            let onset = window.iter().position(|v| v.abs() > 0.5 * peak).unwrap();
            let found = (from + onset) as f64 / OPUS_RATE as f64;
            assert!((found - at).abs() < 0.001, "click at {at}s decoded at {found:.4}s");
        }
    }
}
