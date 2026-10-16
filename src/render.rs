//! A transition rendered to a file, to judge by ear what the scores claim.
//!
//! The incoming phrase starts on the outgoing one's first beat, both eight
//! bars, and the crossfade runs over them, equal-power. The incoming track is
//! resampled onto the playing one's tempo, which moves its pitch with it the
//! way a turntable does: at most a semitone at ±6%. Its level is matched to
//! the playing track's loudness. Mono, as the analysis is.

use crate::library::Track;
use crate::transitions::Transition;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use stelly_audio::decode::{resample, Pcm};

pub struct Rendered {
    pub pcm: Pcm,
    /// Where the crossfade starts and ends in the rendered audio, seconds.
    pub fade: (f64, f64),
}

/// `lead` seconds of the playing track before the crossfade, and as much of
/// the incoming one after it.
pub fn render(playing: &Track, a: &Pcm, next: &Track, b: &Pcm, transition: &Transition, lead: f64) -> Result<Rendered> {
    let rate = a.rate;
    let (out, into) = (&playing.segments[transition.out], &next.segments[transition.into]);

    // The incoming track slowed or sped by the tempo ratio: one second of it
    // becomes `ratio` seconds here.
    let ratio = transition.ratio;
    let stretched_rate = (rate as f64 * ratio).round() as u32;
    let b = resample(&b.samples, b.rate, stretched_rate);
    let gain = match (playing.analysis.loudness, next.analysis.loudness) {
        (Some(a), Some(b)) => 10f64.powf((a - b) / 20.0) as f32,
        _ => 1.0,
    };

    let sample = |seconds: f64| (seconds * rate as f64).round() as isize;
    let start = sample((out.start - lead).max(0.0));
    let fade_from = sample(out.start);
    let fade_to = sample(out.end);
    // Where the incoming phrase starts in its stretched samples.
    let into_at = sample(into.start * ratio);
    let end = fade_to + sample(lead);

    let mut samples = Vec::with_capacity((end - start) as usize);
    for t in start..end {
        let fade = ((t - fade_from) as f64 / (fade_to - fade_from).max(1) as f64).clamp(0.0, 1.0);
        let (fade_out, fade_in) = ((fade * std::f64::consts::FRAC_PI_2).cos(), (fade * std::f64::consts::FRAC_PI_2).sin());
        let from_a = usize::try_from(t).ok().and_then(|i| a.samples.get(i)).copied().unwrap_or(0.0);
        let from_b = usize::try_from(t - fade_from + into_at)
            .ok()
            .and_then(|i| b.get(i))
            .copied()
            .unwrap_or(0.0);
        let a_part = if t < fade_from { from_a } else { from_a * fade_out as f32 };
        let b_part = if t < fade_from { 0.0 } else { from_b * gain * fade_in as f32 };
        samples.push(a_part + b_part);
    }

    // Leave headroom rather than clip where both are at full level.
    let peak = samples.iter().fold(0f32, |m, v| m.max(v.abs()));
    if peak > 0.99 {
        samples.iter_mut().for_each(|v| *v *= 0.99 / peak);
    }
    let seconds = |s: isize| (s - start) as f64 / rate as f64;
    Ok(Rendered {
        pcm: Pcm { samples, rate },
        fade: (seconds(fade_from), seconds(fade_to)),
    })
}

/// 16-bit mono WAV.
pub fn write_wav(path: &Path, pcm: &Pcm) -> Result<()> {
    let data = (pcm.samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&pcm.rate.to_le_bytes());
    out.extend_from_slice(&(pcm.rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for v in &pcm.samples {
        out.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(&out))
        .with_context(|| format!("writing {}", path.display()))
}
