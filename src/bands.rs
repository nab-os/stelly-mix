//! What each beat sounds like: log energy in a dozen bands and a chroma,
//! averaged over the beat. Bars and phrases are found where these change.

use realfft::RealFftPlanner;

const N_FFT: usize = 4096;
const HOP: usize = 1024;
const BANDS: usize = 12;
/// Chroma only from where pitch is clear: above the kick's mud, below the hats.
const CHROMA_HZ: (f64, f64) = (100.0, 5_000.0);

pub const DIMS: usize = BANDS + 12;

/// One row per beat interval, `beats.len() - 1` of them, each column z-scored
/// over the track so that no band drowns the others by being louder.
pub fn per_beat(audio: &[f32], rate: u32, beats: &[f64]) -> Vec<[f64; DIMS]> {
    if beats.len() < 2 || audio.len() < N_FFT {
        return Vec::new();
    }
    let frames = spectrogram(audio, rate);
    let centre = |f: usize| (f * HOP + N_FFT / 2) as f64 / rate as f64;

    let mut rows: Vec<[f64; DIMS]> = beats
        .windows(2)
        .map(|w| {
            let (from, to) = (w[0], w[1]);
            let first = ((from * rate as f64 - (N_FFT / 2) as f64) / HOP as f64).ceil().max(0.0) as usize;
            let inside: Vec<&[f64; DIMS]> = (first..frames.len())
                .take_while(|&f| centre(f) < to)
                .filter(|&f| centre(f) >= from)
                .map(|f| &frames[f])
                .collect();
            let mut row = [0f64; DIMS];
            if inside.is_empty() {
                if let Some(nearest) = frames.get(first.min(frames.len() - 1)) {
                    row = *nearest;
                }
            } else {
                for frame in &inside {
                    for (r, v) in row.iter_mut().zip(frame.iter()) {
                        *r += v / inside.len() as f64;
                    }
                }
            }
            row
        })
        .collect();

    for column in 0..DIMS {
        let n = rows.len() as f64;
        let mean = rows.iter().map(|r| r[column]).sum::<f64>() / n;
        let std = (rows.iter().map(|r| (r[column] - mean).powi(2)).sum::<f64>() / n).sqrt();
        for row in &mut rows {
            row[column] = if std > 1e-9 { (row[column] - mean) / std } else { 0.0 };
        }
    }
    rows
}

/// Log band energies then a normalised chroma, per STFT frame.
fn spectrogram(audio: &[f32], rate: u32) -> Vec<[f64; DIMS]> {
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(N_FFT);
    let window: Vec<f64> = (0..N_FFT)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N_FFT as f64).cos())
        .collect();
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();

    let bin_hz = rate as f64 / N_FFT as f64;
    let (low, high) = (30f64.ln(), 16_000f64.min(rate as f64 / 2.0).ln());
    let edges: Vec<usize> = (0..=BANDS)
        .map(|i| ((low + (high - low) * i as f64 / BANDS as f64).exp() / bin_hz).round() as usize)
        .map(|b| b.min(N_FFT / 2))
        .collect();
    let pitch_class: Vec<Option<usize>> = (0..=N_FFT / 2)
        .map(|bin| {
            let hz = bin as f64 * bin_hz;
            (hz >= CHROMA_HZ.0 && hz <= CHROMA_HZ.1)
                .then(|| ((12.0 * (hz / 440.0).log2()).round() as i64 + 9).rem_euclid(12) as usize)
        })
        .collect();

    let count = (audio.len() - N_FFT) / HOP + 1;
    let mut frames = Vec::with_capacity(count);
    for f in 0..count {
        let start = f * HOP;
        for (i, slot) in input.iter_mut().enumerate() {
            *slot = audio[start + i] as f64 * window[i];
        }
        fft.process_with_scratch(&mut input, &mut spectrum, &mut scratch)
            .expect("buffers come from the plan");

        let mut row = [0f64; DIMS];
        for band in 0..BANDS {
            let (lo, hi) = (edges[band], edges[band + 1].max(edges[band] + 1));
            let energy: f64 = spectrum[lo..hi.min(spectrum.len())].iter().map(|c| c.norm_sqr()).sum();
            row[band] = (1e-10 + energy).ln();
        }
        let mut chroma = [0f64; 12];
        for (c, pc) in spectrum.iter().zip(&pitch_class) {
            if let Some(pc) = pc {
                chroma[*pc] += c.norm_sqr();
            }
        }
        let total: f64 = chroma.iter().sum::<f64>().max(1e-12);
        for (slot, v) in row[BANDS..].iter_mut().zip(chroma) {
            *slot = v / total;
        }
        frames.push(row);
    }
    frames
}
