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

## Mixing

```sh
cargo run --release -- index ~/Music/Hard\ Tech            # analyse and embed every phrase
cargo run --release -- next some/track.opus                # where to go from here
cargo run --release -- mix --out t.wav playing.opus next.opus   # hear the best way there
```

`index` cuts every track at its eight-bar phrases and embeds each with
CLAP's audio tower, the same model and weights as Stelly, fetched once
(117MB, hash-checked) into `~/.local/share/stelly-mix/models`. The library is
one JSON file a track in `~/.local/share/stelly-mix/library`, redone when the
file changes. 159 tracks take about two minutes.

`next` ranks the library from the playing track: the best pair of a phrase
in its second half and one in the other track's first, scored on how alike
the two phrases sound (against the other candidates: within a genre every
pair is 0.93–0.96 raw), the tempo change, Camelot steps and the loudness
step. Tracks more than ±6% away are left out.

```
0.72  Remon Verhoeve - High on C
      out at 4:12.0 → in at 0:36.7, -1.9% tempo, 3B → 3B, sound 0.98, -0.5 LU
```

`mix` renders that transition, in stereo, in about seven seconds. The
incoming phrase starts on the outgoing one's first beat and the transition
runs over those eight bars as a bass swap: the incoming track's mids and
highs fade in over the first half with its bass held back, the basses trade
places on the downbeat in the middle (a Linkwitz-Riley split at 150Hz), and
the outgoing track's mids and highs fade out over the second half. The
incoming track is resampled onto the playing one's tempo, which keeps the
beats together, then keylocked back to its own pitch by Signalsmith Stretch.
`--turntable` lets the pitch move instead, `--crossfade` makes it a plain
equal-power fade. Analysed again, a render is one grid from end to end:
99–100% of its beats on it, before, during and after the transition.

`set` plans a whole set from a first track and renders it:

```sh
cargo run --release -- set --minutes 30 --out set.wav first.opus
```

The set keeps the first track's tempo, every track keylocked onto it, so
candidates are ranked by their stretch from that tempo, not from the track
before: a run of 155s would otherwise score as perfect while each plays 3%
fast. A beam of the best partial sets is grown a transition at a time, no
track twice, each track playing at least 90 seconds between its transitions,
and the set with the best average transition wins. Planning 30 minutes over
159 tracks takes under a second; rendering it, a track decoded at a time,
about a minute and a half and 1.5GB. It prints the tracklist with the times
each track comes in. On the first one rendered, from Remon Verhoeve's The
Engine: 12 tracks in 34 minutes, each solo section within -9.9 to -12.3
LUFS, and the 20s windows around every transition 85–100% on one grid, but
one at 70%.

`find` and `set --like` look for a sound by description, through CLAP's text
tower, the same weights as Stelly's text steering:

```sh
cargo run --release -- find "chill relaxed downtempo electronic music"
cargo run --release -- set --like "chill relaxed downtempo electronic music" --out chill.wav
```

A track scores by how much nearer its phrases are to the description than to
plain "music", which on its own mostly measures sounding like music at all,
ranked across the library. "aggressive hard techno" puts the hard tech folder
at a mean rank of 0.80 against 0.38 for the rest; "chill relaxed downtempo"
at 0.33 against 0.57. With `--like`, a set leaves out tracks under the 60th
percentile, counts the rest towards each transition, and starts from the best
match when no first track is given.

Levels are matched the way a DJ trims: each track's main sections, the 80th
percentile of its phrases' stereo loudness, not its average, which a quiet
intro drags down, and not the analysis's mono figure, which reads a wide
master 3–4 dB low. The louder track is turned down to the quieter, and the
whole render sits 3 dB under that: two beat-matched masters peaking at full
scale reach 1.4 in the middle of the blend, and without the room the limiter
pulls every kick of the transition down and leaves it 2 LU under the tracks
either side. A, the transition and B now measure within about 0.8 LU of each
other; peaks are held at -1 dBFS.

Building it wants a C++ compiler and libclang, for Signalsmith Stretch.

## How it works

Decoding and the descriptors (tempo estimate, key, loudness) come from
`stelly-audio`, the analysis crate shared with Stelly. Everything from beats
onwards is here, and so is Opus: symphonia demuxes it, `rusty-opus` decodes
it, pure Rust and sample-for-sample with libopus, pre-skip dropped so beats
are not 6.5ms late.

- **Tempo**: the onset envelope's strongest autocorrelation peaks are each
  tracked, and the one whose beats sit best on a steady grid wins. The
  tallest peak alone picks a triplet pulse on some music (83 for a 125 BPM
  track). Which octave to report is not the tracker's call: hard tech it
  follows cleanly at 77.5 and loses at 155 still has a 155 grid. So the
  winner is moved into a one-octave range, 88–176 by default, by doubling
  its grid or by halving it onto the stronger beats.
- **Beats**: Ellis's dynamic programme over Stelly's onset envelope.
  Frame-accurate, about ±5ms.
- **Grid**: a line fitted through the steadiest 32 beats, then extended to
  every beat near it, so the beats a tracker wandered through a breakdown
  are outliers rather than a miscount. On produced music it lands on
  round tempos (148.00, 174.00) and a phase within a couple of milliseconds,
  which is what syncing two decks needs. A track is *steady* when the
  stretches of it that keep a beat of their own agree with the grid: a long
  breakdown the tracker wanders through has no say, a drifting live take
  agrees only for a while. A steady track's beats are the grid's lines,
  breakdowns included; played music keeps its tracked beats.
- **Downbeats**: per-beat band energies and chroma; the beat position where a
  bar before differs most from a bar after is the downbeat.
- **Phrases and sections**: the same contrast a level up, eight bars at a time.
- **Key**: Krumhansl profiles from `stelly-audio`, written in Camelot.

## Where it stands

Measured against Ballroom's annotations and against beat_this on 159 hard
tech tracks and 40 others (below), and on click tracks and a synthetic song
in the tests.

- **Downbeats follow the beats.** Where the beats are right the bar
  contrast picks the right downbeat on 96% of hard tech (27 of 28 tracks
  where beat_this's own downbeats are consistent). Kick, kick-minus-snare,
  harmonic change and their sums did no better in a trial on 384 tracks.
  On Ballroom's 30s Latin clips it is 60%.
- **The octave is taste.** 88–176 suits hard tech, techno, house, drum &
  bass and hip hop at 90; frenchcore wants `--bpm-range 100-200`, and slow
  hip hop at 85 comes out at 170. Ballroom's waltzes are doubled by it.
- **Some tracks still tie their tempo to the wrong pulse**, 3:2 or the
  off-beat: the right tempo is among the candidates for 96% of hard tech,
  and chosen for 88%.
- **Beats sit ~6ms late** of Ballroom's annotations (median; quartiles 4 and
  9ms), well inside the tolerance and under a frame.

## Evaluation

```sh
git clone https://github.com/CPJKU/BallroomAnnotations data/ballroom/annotations
curl -L https://mtg.upf.edu/ismir2004/contest/tempoContest/data1.tar.gz | tar xz -C data/ballroom
cargo run --release -- eval --worst 15 data/ballroom/annotations data/ballroom/BallroomData
```

F-measure at ±70ms for beats and downbeats, Acc1 (tempo within 4%) and Acc2
(also forgiving ×2, ×½, ×3, ×⅓):

| against                          | files | beat F | down F | Acc1  | Acc2  |
|----------------------------------|------:|-------:|-------:|------:|------:|
| Ballroom, annotated              |   698 |  0.755 |  0.473 | 0.599 | 0.887 |
| beat_this, hard tech             |   159 |  0.846 |  0.623 | 0.893 | 0.931 |
| beat_this, 40 FLACs, all genres  |    40 |  0.704 |  0.504 | 0.575 | 0.950 |

Published trackers reach about 0.9 beat F on Ballroom. Some of the gap is
the octave policy, not the audio: its Quickstep is ~200 BPM and halved, its
slow dances doubled, and the waltzes are in 3/4, which neither the folding
nor the four-beat bars know about.

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
2. Transitions tuned by ear: the weights are a first guess.
3. Sets that go somewhere: an energy arc, a tempo that moves, a key path.
4. Two-deck playback, live.

## Sources

Tracks are files. Anything else, a streaming service or a server, is a
`Source` (`src/source.rs`) that turns its own references, `name:rest`, into
files in `~/.cache/stelly-mix/<name>`; every command that takes a file takes a
reference too, and `search <name> <query>` lists them. This build registers
none, so it carries no service's client, credentials or terms. A build that
wants one is its own crate, with a `main` that passes it in:

```rust
fn main() -> anyhow::Result<()> {
    stelly_mix::cli::run(vec![Box::new(MyService::from_env()?)])
}
```

## Building

A Rust toolchain, a C++ compiler and libclang (for Signalsmith Stretch).
`stelly-audio`, the analysis shared with Stelly, comes from its own
repository, and CLAP's weights are fetched on first use (117MB for the audio
tower, 500MB more for the text tower if `find` or `--like` is used).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
