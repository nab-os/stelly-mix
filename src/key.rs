//! Keys the way DJs read them: the Camelot wheel, where a step round the
//! wheel or across between major and minor is a compatible mix.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Key {
    /// Steps between two keys on the wheel: 0 the same key, 1 a fifth either
    /// way or the relative major or minor, the classic safe moves, more
    /// beyond. Crossing between rings costs one step on top of the turn.
    pub fn distance(&self, other: &Key) -> Option<u32> {
        let (a, ring_a) = parse(&self.camelot)?;
        let (b, ring_b) = parse(&other.camelot)?;
        let turn = (a + 12 - b) % 12;
        Some(turn.min(12 - turn) + u32::from(ring_a != ring_b))
    }
}

fn parse(camelot: &str) -> Option<(u32, char)> {
    let ring = camelot.chars().last()?;
    Some((camelot[..camelot.len() - 1].parse().ok()?, ring))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_round_the_wheel() {
        let key = |t, s| Key::new(t, s, 1.0).unwrap();
        let a_minor = key("A", "minor");
        assert_eq!(a_minor.distance(&a_minor), Some(0));
        assert_eq!(a_minor.distance(&key("E", "minor")), Some(1)); // 8A -> 9A
        assert_eq!(a_minor.distance(&key("C", "major")), Some(1)); // 8A -> 8B
        assert_eq!(a_minor.distance(&key("B", "minor")), Some(2)); // 8A -> 10A
        assert_eq!(key("G#", "minor").distance(&key("B", "minor")), Some(3)); // 1A -> 10A, the short way
    }

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
