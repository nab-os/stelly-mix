//! A transition rendered to a file, to judge by ear what the scores claim.
//!
//! The incoming phrase starts on the outgoing one's first beat, both eight
//! bars, and the transition runs over them. The incoming track is resampled
//! onto the playing one's tempo, which is what keeps the beats together, then
//! with keylock its pitch is put back where it was by Signalsmith Stretch;
//! without, it moves as on a turntable, at most a semitone at ±6%. The
//! louder of the two is turned down to the other's loudness.
//!
//! The transition is a bass swap, as a DJ does it on a mixer's EQ: the two
//! tracks' mids and highs cross over the phrase, equal-power, while the
//! incoming bass is held back until the downbeat in the middle, where the
//! basses trade places. Two kicks and two basslines never play over each
//! other. Or, with `bass_swap` off, a plain equal-power crossfade. The whole
//! render sits 3 dB under the quieter track, so the transition, where both
//! play, has room, and a limiter catches what peaks remain.

use crate::decode::Stereo;
use crate::library::Track;
use crate::transitions::Transition;
use anyhow::{Context, Result};
use std::f64::consts::{FRAC_PI_2, PI, SQRT_2};
use std::io::Write;
use std::path::Path;
use stelly_audio::decode::resample;

/// Where the bass is split from the rest.
const CROSSOVER_HZ: f64 = 150.0;
/// How long the basses take to trade places: short enough to land on the
/// beat, long enough not to click.
const SWAP_SECONDS: f64 = 0.03;
/// How far under the quieter track the render is set. Masters peak at full
/// scale, and two of them beat-matched, at the middle of an equal-power
/// blend, reach 1.4: 3 dB is room enough that the limiter only touches the
/// odd peak, rather than pulling every kick of the transition down and
/// leaving it 2 LU under the tracks either side.
const HEADROOM_DB: f64 = 3.0;
/// Peak level the limiter holds the render to: -1 dBFS.
const CEILING: f32 = 0.891;
const LOOKAHEAD: f64 = 0.005;
const RELEASE: f64 = 0.1;

#[derive(Debug, Clone)]
pub struct Settings {
    /// Seconds of each track around the transition.
    pub lead: f64,
    pub keylock: bool,
    pub bass_swap: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lead: 30.0,
            keylock: true,
            bass_swap: true,
        }
    }
}

pub struct Rendered {
    pub audio: Stereo,
    /// Where the transition starts and ends in the rendered audio, seconds.
    pub fade: (f64, f64),
    /// Where the basses trade places, seconds, with a bass swap.
    pub swap: Option<f64>,
}

pub fn render(
    playing: &Track,
    a: &Stereo,
    next: &Track,
    b: &Stereo,
    transition: &Transition,
    settings: &Settings,
) -> Result<Rendered> {
    let rate = a.rate;
    let (out, into) = (&playing.segments[transition.out], &next.segments[transition.into]);
    let sample = |seconds: f64| (seconds * rate as f64).round() as isize;

    let start = sample((out.start - settings.lead).max(0.0));
    let (fade_from, fade_to) = (sample(out.start), sample(out.end));
    let end = fade_to + sample(settings.lead);
    // The downbeat nearest the middle of the phrase.
    let middle = (out.start + out.end) / 2.0;
    let swap_at = playing
        .analysis
        .downbeats
        .iter()
        .copied()
        .filter(|d| *d > out.start && *d < out.end)
        .min_by(|x, y| (x - middle).abs().total_cmp(&(y - middle).abs()))
        .unwrap_or(middle);
    let swap = sample(swap_at);

    // Both brought to the quieter one's loudness, less the headroom: turning
    // the hotter track down, as a DJ does with the trim. Boosting the quieter one instead
    // would drive a master already peaking at full scale into the limiter on
    // every kick, which takes it back down, and unevenly.
    let (a_gain, b_gain) = match (body_loudness(a, playing), body_loudness(b, next)) {
        (Some(a), Some(b)) => {
            let target = a.min(b) - HEADROOM_DB;
            (db(target - a), db(target - b))
        }
        _ => (1.0, 1.0),
    };

    // The incoming track from its phrase on, on the playing track's tempo:
    // one second of it lasts `ratio` seconds here.
    let ratio = transition.ratio;
    let needed = (end - fade_from) as usize;
    let b = stretch(b, rate, ratio, settings.keylock, into.start, needed);

    let length = (end - start) as usize;
    let mut channels = Vec::with_capacity(2);
    for (a_channel, b_channel) in [(&a.left, &b.0), (&a.right, &b.1)] {
        let a_part: Vec<f32> = (start..end)
            .map(|t| usize::try_from(t).ok().and_then(|i| a_channel.get(i)).map_or(0.0, |v| v * a_gain))
            .collect();
        let b_part: Vec<f32> = (start..end)
            .map(|t| {
                usize::try_from(t - fade_from)
                    .ok()
                    .and_then(|i| b_channel.get(i))
                    .map_or(0.0, |v| v * b_gain)
            })
            .collect();
        let (a_low, b_low) = if settings.bass_swap {
            (lowpass(&a_part, rate), lowpass(&b_part, rate))
        } else {
            (vec![0.0; length], vec![0.0; length])
        };

        let mixed: Vec<f32> = (0..length)
            .map(|i| {
                let t = start + i as isize;
                let (a_gains, b_gains) = if settings.bass_swap {
                    swap_gains(t, fade_from, fade_to, swap, rate)
                } else {
                    let fade = progress(t, fade_from, fade_to);
                    let (a, b) = ((fade * FRAC_PI_2).cos(), (fade * FRAC_PI_2).sin());
                    ((a, a), (b, b))
                };
                let (a_high, b_high) = (a_part[i] - a_low[i], b_part[i] - b_low[i]);
                (a_low[i] as f64 * a_gains.0
                    + a_high as f64 * a_gains.1
                    + b_low[i] as f64 * b_gains.0
                    + b_high as f64 * b_gains.1) as f32
            })
            .collect();
        channels.push(mixed);
    }
    let right = channels.pop().unwrap_or_default();
    let mut left = channels.pop().unwrap_or_default();
    let mut right = right;

    limit(&mut left, &mut right, rate);
    let seconds = |s: isize| (s - start) as f64 / rate as f64;
    Ok(Rendered {
        audio: Stereo { left, right, rate },
        fade: (seconds(fade_from), seconds(fade_to)),
        swap: settings.bass_swap.then(|| seconds(swap)),
    })
}

/// (bass, rest) gains of the outgoing and incoming tracks at sample `t`.
fn swap_gains(t: isize, from: isize, to: isize, swap: isize, rate: u32) -> ((f64, f64), (f64, f64)) {
    let ramp = (SWAP_SECONDS * rate as f64) as isize;
    // The basses trade over `ramp` samples ending on the beat.
    let bass_in = progress(t, swap - ramp, swap);
    let (bass_in, bass_out) = ((bass_in * FRAC_PI_2).sin(), (bass_in * FRAC_PI_2).cos());
    // The rest equal-power over the whole phrase: two tracks' mids and highs
    // add as power, so the level holds through it; crossed linearly, it dips
    // 2 LU in the middle.
    let rest = progress(t, from, to) * FRAC_PI_2;
    ((bass_out, rest.cos()), (bass_in, rest.sin()))
}

/// Peaks brought down to `CEILING` and nothing else touched: the tracks are
/// mastered loud, and turning the whole render down to fit the odd peak where
/// two of them meet would leave it quieter than either. Stereo-linked, so the
/// image does not lean. The gain is the lowest needed over the next
/// `LOOKAHEAD`, averaged over the last `LOOKAHEAD`: that reaches the needed
/// gain by the time the peak does without stepping, then recovers over
/// `RELEASE`.
fn limit(left: &mut [f32], right: &mut [f32], rate: u32) {
    let ahead = ((LOOKAHEAD * rate as f64) as usize).max(1);
    let recover = 1.0 - (-1.0 / (RELEASE * rate as f64)).exp() as f32;
    let needed: Vec<f32> = left
        .iter()
        .zip(right.iter())
        .map(|(l, r)| (CEILING / l.abs().max(r.abs()).max(1e-9)).min(1.0))
        .collect();
    let n = needed.len();

    // Lowest over [i, i + ahead), by a monotonic queue.
    let mut held = vec![1f32; n];
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for i in (0..n).rev() {
        while queue.back().is_some_and(|&j| needed[j] >= needed[i]) {
            queue.pop_back();
        }
        queue.push_back(i);
        while queue.front().is_some_and(|&j| j >= i + ahead) {
            queue.pop_front();
        }
        held[i] = needed[*queue.front().expect("just pushed")];
    }

    let (mut sum, mut gain) = (0f64, 1f32);
    for i in 0..n {
        sum += held[i] as f64;
        if i >= ahead {
            sum -= held[i - ahead] as f64;
        }
        let smoothed = (sum / (i + 1).min(ahead) as f64) as f32;
        gain = smoothed.min(gain + (1.0 - gain) * recover);
        left[i] *= gain;
        right[i] *= gain;
    }
}

/// How loud a track's main sections are, in stereo: the 80th percentile of
/// its phrases' loudness. Not the whole track's, which its intro and its
/// breakdowns pull down by however long they last; and not the analysis's
/// mono figure, which reads a wide master 3–4 dB low, by how wide it is.
fn body_loudness(audio: &Stereo, track: &Track) -> Option<f64> {
    let rate = audio.rate as f64;
    let mut phrases: Vec<f64> = track
        .segments
        .iter()
        .filter_map(|segment| {
            let from = ((segment.start * rate) as usize).min(audio.len());
            let to = ((segment.end * rate) as usize).min(audio.len());
            let mut meter = ebur128::EbuR128::new(2, audio.rate, ebur128::Mode::I).ok()?;
            meter.add_frames_planar_f32(&[&audio.left[from..to], &audio.right[from..to]]).ok()?;
            meter.loudness_global().ok().filter(|l| l.is_finite())
        })
        .collect();
    phrases.sort_by(f64::total_cmp);
    phrases.get((phrases.len() * 4 / 5).min(phrases.len().saturating_sub(1))).copied()
}

/// A level change in dB as a gain.
fn db(change: f64) -> f32 {
    10f64.powf(change / 20.0) as f32
}

/// How far `t` is from `from` to `to`, 0..1.
fn progress(t: isize, from: isize, to: isize) -> f64 {
    ((t - from) as f64 / (to - from).max(1) as f64).clamp(0.0, 1.0)
}

/// `len` samples of each channel of `b` from `from` seconds into it, at
/// `rate`, slowed or sped so one second lasts `ratio`, and with `keylock` at
/// its own pitch. Only what is needed is resampled: a whole track at an odd
/// ratio takes the slow path through the resampler.
fn stretch(b: &Stereo, rate: u32, ratio: f64, keylock: bool, from: f64, len: usize) -> (Vec<f32>, Vec<f32>) {
    let stretched_rate = (rate as f64 * ratio).round() as u32;
    // Half a second either side, for the resampler's and the pitch shifter's
    // edges to settle, cut off after.
    let margin = (b.rate / 2) as usize;
    let at = (from * b.rate as f64).round() as usize;
    let lo = at.saturating_sub(margin);
    let hi = (at + (len as f64 / ratio * b.rate as f64 / rate as f64) as usize + margin).min(b.len());
    // Where `from` lands once resampled.
    let lead = ((at - lo) as f64 * stretched_rate as f64 / b.rate as f64).round() as usize;
    let source = |channel: &[f32]| resample(channel.get(lo..hi.max(lo)).unwrap_or_default(), b.rate, stretched_rate);
    let (left, right) = (source(&b.left), source(&b.right));
    let (left, right) = if keylock && (ratio - 1.0).abs() > 1e-4 {
        transpose(&left, &right, rate, ratio)
    } else {
        (left, right)
    };
    let cut = |channel: Vec<f32>| channel.into_iter().skip(lead).take(len).collect();
    (cut(left), cut(right))
}

/// Pitch times `factor`, length unchanged.
fn transpose(left: &[f32], right: &[f32], rate: u32, factor: f64) -> (Vec<f32>, Vec<f32>) {
    let mut stretcher = signalsmith_stretch::Stretch::preset_default(2, rate);
    stretcher.set_transpose_factor(factor as f32, None);
    let input: Vec<f32> = left.iter().zip(right).flat_map(|(l, r)| [*l, *r]).collect();
    let mut output = vec![0f32; input.len()];
    stretcher.exact(&input, &mut output);
    output.chunks_exact(2).map(|lr| (lr[0], lr[1])).unzip()
}

/// Linkwitz-Riley low-pass at `CROSSOVER_HZ`: two Butterworth sections. The
/// rest is the signal minus this, so the two bands always add back up to
/// exactly the signal.
fn lowpass(input: &[f32], rate: u32) -> Vec<f32> {
    let w = 2.0 * PI * CROSSOVER_HZ / rate as f64;
    let alpha = w.sin() / (2.0 * (1.0 / SQRT_2));
    let a0 = 1.0 + alpha;
    let (b0, b1, b2) = ((1.0 - w.cos()) / 2.0 / a0, (1.0 - w.cos()) / a0, (1.0 - w.cos()) / 2.0 / a0);
    let (a1, a2) = (-2.0 * w.cos() / a0, (1.0 - alpha) / a0);
    let section = |x: &[f64]| -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&x0| {
                let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                (x2, x1, y2, y1) = (x1, x0, y1, y0);
                y0
            })
            .collect()
    };
    let once = section(&input.iter().map(|&v| v as f64).collect::<Vec<_>>());
    section(&once).into_iter().map(|v| v as f32).collect()
}

/// 16-bit stereo WAV.
pub fn write_wav(path: &Path, audio: &Stereo) -> Result<()> {
    let data = (audio.len() * 4) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&audio.rate.to_le_bytes());
    out.extend_from_slice(&(audio.rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for (l, r) in audio.left.iter().zip(&audio.right) {
        for v in [l, r] {
            out.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(&out))
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    fn sine(hz: f64, seconds: f64) -> Vec<f32> {
        (0..(seconds * RATE as f64) as usize)
            .map(|i| (0.5 * (2.0 * PI * hz * i as f64 / RATE as f64).sin()) as f32)
            .collect()
    }

    /// Frequency from upward zero crossings over the middle of a signal.
    fn frequency(signal: &[f32]) -> f64 {
        let middle = &signal[signal.len() / 4..3 * signal.len() / 4];
        let crossings = middle.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        crossings as f64 / (middle.len() as f64 / RATE as f64)
    }

    #[test]
    fn keylock_keeps_the_pitch_and_the_turntable_does_not() {
        let tone = sine(440.0, 4.0);
        let b = Stereo {
            left: tone.clone(),
            right: tone,
            rate: RATE,
        };
        // Sped up by 5%: one second of it lasts 0.95 here.
        let len = (3.0 * 0.95 * RATE as f64) as usize;
        let (locked, _) = stretch(&b, RATE, 0.95, true, 0.0, len);
        let (turntable, _) = stretch(&b, RATE, 0.95, false, 0.0, len);
        assert_eq!(locked.len(), len);
        assert!((frequency(&locked) - 440.0).abs() < 3.0, "keylock: {} Hz", frequency(&locked));
        assert!((frequency(&turntable) - 440.0 / 0.95).abs() < 3.0, "turntable: {} Hz", frequency(&turntable));
    }

    #[test]
    fn the_low_band_holds_the_bass_and_not_the_rest() {
        // RMS after the filter has settled, over RMS before.
        let kept = |hz: f64| {
            let tone = sine(hz, 1.0);
            let low = lowpass(&tone, RATE);
            let rms = |v: &[f32]| (v[RATE as usize / 2..].iter().map(|x| x * x).sum::<f32>()).sqrt();
            rms(&low) / rms(&tone)
        };
        assert!(kept(50.0) > 0.9, "50Hz: {}", kept(50.0));
        assert!(kept(1000.0) < 0.01, "1kHz: {}", kept(1000.0));
    }

    #[test]
    fn the_basses_trade_on_the_beat_and_the_level_holds() {
        let (from, to, swap) = (0, 8 * RATE as isize, 4 * RATE as isize);
        let ((a_bass, _), (b_bass, _)) = swap_gains(swap - RATE as isize, from, to, swap, RATE);
        assert_eq!((a_bass, b_bass), (1.0, 0.0));
        let ((a_bass, _), (b_bass, _)) = swap_gains(swap, from, to, swap, RATE);
        assert!(a_bass.abs() < 1e-9 && (b_bass - 1.0).abs() < 1e-9);
        for t in (from..=to).step_by(RATE as usize / 4) {
            let ((_, a_rest), (_, b_rest)) = swap_gains(t, from, to, swap, RATE);
            assert!((a_rest * a_rest + b_rest * b_rest - 1.0).abs() < 1e-9, "power at {t}");
        }
    }

    #[test]
    fn the_limiter_holds_the_ceiling_and_leaves_the_rest() {
        let mut left: Vec<f32> = sine(100.0, 2.0).iter().map(|v| v * 1.2).collect();
        // A quiet second after a loud one.
        for v in &mut left[RATE as usize..] {
            *v *= 0.25;
        }
        let mut right = left.clone();
        let quiet = left[RATE as usize * 3 / 2];
        limit(&mut left, &mut right, RATE);
        let peak = left.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak <= CEILING + 1e-6, "peak {peak}");
        assert!((left[RATE as usize * 3 / 2] - quiet).abs() < 1e-3, "the quiet part was touched");
    }

}
