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
onwards is here, and so is Opus: symphonia demuxes it, `rusty-opus` decodes
it, pure Rust and sample-for-sample with libopus, pre-skip dropped so beats
are not 6.5ms late.

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

Measured against the Ballroom set (below), on click tracks and a synthetic
song in the tests, and run on a handful of real tracks.

- **Downbeats are weak on real music.** Confidence comes out at 0.02–0.11:
  loops look the same from every beat position. Needs better cues (kick vs.
  snare pattern, harmonic change) and an annotated set to measure against.
- **The tempo octave is a guess.** Tempos are reported in 75–175 BPM by
  default, `--bpm-range 100-220` for frenchcore. Inside the range, grid share
  favours the faster pulse: with `--bpm-range 60-240` Ballroom's Quickstep
  goes from 0.63 to 0.81 beat F but Samba and Rumba get doubled, 0.79 overall
  against 0.77. And a hard tech track can track cleanly at 78 and not at 156.
- **Beats sit ~6ms late** of Ballroom's annotations (median; quartiles 4 and
  9ms), well inside the tolerance and under a frame.

## Evaluation

```sh
git clone https://github.com/CPJKU/BallroomAnnotations data/ballroom/annotations
curl -L https://mtg.upf.edu/ismir2004/contest/tempoContest/data1.tar.gz | tar xz -C data/ballroom
cargo run --release -- eval --worst 15 data/ballroom/annotations data/ballroom/BallroomData
```

F-measure at ±70ms for beats and downbeats, Acc1 (tempo within 4%) and Acc2
(also forgiving ×2, ×½, ×3, ×⅓). Ballroom, 698 clips of 30s:

|                     | files | beat F | down F | Acc1  | Acc2  |
|---------------------|------:|-------:|-------:|------:|------:|
| ChaChaCha           |   111 |  0.840 |  0.359 | 0.910 | 0.910 |
| Jive                |    60 |  0.771 |  0.260 | 0.517 | 0.967 |
| Quickstep           |    82 |  0.626 |  0.436 | 0.000 | 0.963 |
| Rumba-American      |     7 |  0.816 |  0.554 | 1.000 | 1.000 |
| Rumba-International |    51 |  0.866 |  0.703 | 0.941 | 1.000 |
| Rumba-Misc          |    40 |  0.791 |  0.630 | 0.850 | 0.975 |
| Samba               |    86 |  0.882 |  0.575 | 0.930 | 0.942 |
| Tango               |    86 |  0.886 |  0.626 | 0.942 | 0.942 |
| VienneseWaltz       |    65 |  0.555 |  0.326 | 0.031 | 0.615 |
| Waltz               |   110 |  0.722 |  0.306 | 0.591 | 0.818 |
| **all**             |   698 |  0.773 |  0.450 | 0.643 | 0.898 |

Published trackers reach about 0.9 beat F on this set. Some of the gap is
deliberate: Quickstep, ~200 BPM, is reported at half tempo by the 75–175
range, and the waltzes are in 3/4, which neither the halving nor the
four-beat bars know about. The worst files have the right tempo and the
wrong phase, on the off-beat of syncopated Latin rhythms.

### Against another tracker

Ballroom is no hard tech, and no one has annotated a hard tech set. So
`tools/reference.py` runs beat_this (CPJKU, ISMIR 2024, the state of the
art) over a folder and writes its beats in the same `.beats` format:

```sh
uv run tools/reference.py ~/Music/Hard\ Tech data/reference/hardtech
cargo run --release -- eval --worst 20 data/reference/hardtech ~/Music/Hard\ Tech
```

That is agreement, not accuracy: where the two disagree is where to listen.
beat_this was trained on Ballroom, so it is no referee there, and without
its DBN its beats sit on a 20ms grid, too coarse to judge a grid's phase.
CPU only, about 9s for a five-minute track.

## Diagnostics

`cargo run --release --example tempi <file> [bpm...]` shows each tempo
candidate and how the tracker does with it; `--example sections <file> <bpm>`
fits a grid per 20s window, to see where a track drifts.

## Next

1. Better than the baseline above: beat phase on syncopated music, metres
   other than 4/4, downbeats. And a set nearer what DJs play: Ballroom is
   no hard tech.
2. Segments: CLAP embeddings per phrase, so passages can be compared, not
   just tracks.
3. Transitions: from track A's phrase to track B's, scored on segment
   distance, tempo ratio, Camelot distance and energy.
4. Two-deck playback with beat-synced crossfades.

## Building

`stelly-audio` comes from the private Stelly repository over SSH, which is
why `.cargo/config.toml` hands git to the git CLI. It points at the
`shared-audio-crate` branch until that is merged.
