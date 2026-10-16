# stelly-mix

Stelly's sibling for people who mix. Where Stelly turns whole tracks into
points in a space, stelly-mix cares about time inside a track: where the beats
are, whether they sit on a grid, where bars and phrases start, and what key it
is in. That is what a transition is built from.

```sh
cargo run --release -- analyse ~/Music/some/track.flac
cargo run --release -- analyse --json *.flac     # every beat, one object per file
```

```
Library/ALT BLK ERA/Misfits _ SOLAR/02 - I'm Normally Like This.flac
  148.00 BPM, steady (93% of beats on the grid), first beat at 0.027s
  5A (C minor, strength 0.73)
  first downbeat at 3.265s (confidence 0.08), 102 bars, 12 phrases
  sections at 0:13.0, 0:38.9, 1:04.9, 1:17.9, 1:30.8, 2:22.7
  2:54.1 long, analysed in 0.4s
```

## How it works

Decoding and the descriptors (tempo estimate, key, loudness) come from
`stelly-audio`, the analysis crate shared with Stelly. Everything from beats
onwards is here.

- **Tempo**: the onset envelope's strongest autocorrelation peaks are each
  tracked, and the one whose beats sit best on a steady grid wins. The
  tallest peak alone picks a triplet pulse on some music (83 for a 125 BPM
  track). The winner is then brought into 75–175 BPM by doubling, or by
  halving the grid onto its stronger beats: tracking a rock song directly at
  112 latches onto the backbeat, while its hi-hats give a clean 224 grid.
- **Beats**: Ellis's dynamic programme over Stelly's onset envelope.
  Frame-accurate, about ±5ms.
- **Grid**: a line fitted through the beats. On produced music it lands on
  round tempos (148.00, 174.00) and a phase within a couple of milliseconds,
  which is what syncing two decks needs. A track is *steady* when 80% of its
  beats sit within 20ms of the grid; played music drifts, and should be
  synced from its beats instead.
- **Downbeats**: per-beat band energies and chroma; the beat position where a
  bar before differs most from a bar after is the downbeat.
- **Phrases and sections**: the same contrast a level up, eight bars at a time.
- **Key**: Krumhansl profiles from `stelly-audio`, written in Camelot.

## Where it stands

Measured on click tracks and a synthetic song (see the tests), and run on a
handful of real tracks. Not yet measured against ground truth.

- **Downbeats are weak on real music.** Confidence comes out at 0.02–0.11:
  loops look the same from every beat position. Needs better cues (kick vs.
  snare pattern, harmonic change) and an annotated set to measure against.
- **Tempo range is fixed** at 75–175 BPM. Frenchcore at 200 reads as 100;
  it should be a setting.
- **No Opus.** symphonia has no Opus decoder, and Opus was a fifth of the
  library this was first tried on.

## Diagnostics

`cargo run --release --example tempi <file> [bpm...]` shows each tempo
candidate and how the tracker does with it; `--example sections <file> <bpm>`
fits a grid per 20s window, to see where a track drifts.

## Next

1. Ground truth: beat and downbeat annotations (GTZAN/Ballroom/Harmonix
   sets, or a Mixxx/Rekordbox library's grids) and an `eval` command.
2. Opus, through libopus or an ffmpeg fallback.
3. Segments: CLAP embeddings per phrase, so passages can be compared, not
   just tracks.
4. Transitions: from track A's phrase to track B's, scored on segment
   distance, tempo ratio, Camelot distance and energy.
5. Two-deck playback with beat-synced crossfades.

## Building

`stelly-audio` comes from the private Stelly repository over SSH, which is
why `.cargo/config.toml` hands git to the git CLI. It points at the
`shared-audio-crate` branch until that is merged.
