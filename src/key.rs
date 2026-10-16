//! Keys the way DJs read them: the Camelot wheel, where a step round the
//! wheel or across between major and minor is a compatible mix.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Key {
    /// Pitch class with sharps, "C" .. "B".
    pub tonic: String,
    /// "major" or "minor".
    pub scale: String,
    /// "8A" for A minor, "8B" for C major.
    pub camelot: String,
    /// Correlation with the key profile, 0..1.
    pub strength: f64,
}

const PITCH_CLASSES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

impl Key {
    pub fn new(tonic: &str, scale: &str, strength: f64) -> Option<Self> {
        let pc = PITCH_CLASSES.iter().position(|&p| p == tonic)?;
        // C major is 8B and each fifth up is one more; a minor sits with its
        // relative major, three semitones up.
        let (major_pc, letter) = match scale {
            "major" => (pc, 'B'),
            "minor" => ((pc + 3) % 12, 'A'),
            _ => return None,
        };
        let number = (major_pc * 7 % 12 + 7) % 12 + 1;
        Some(Self {
            tonic: tonic.to_string(),
            scale: scale.to_string(),
            camelot: format!("{number}{letter}"),
            strength,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camelot_numbers() {
        for (tonic, scale, camelot) in [
            ("C", "major", "8B"),
            ("A", "minor", "8A"),
            ("G", "major", "9B"),
            ("F", "major", "7B"),
            ("B", "major", "1B"),
            ("F#", "minor", "11A"),
            ("G#", "minor", "1A"),
            ("D", "minor", "7A"),
        ] {
            assert_eq!(Key::new(tonic, scale, 1.0).unwrap().camelot, camelot, "{tonic} {scale}");
        }
    }
}
