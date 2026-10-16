# /// script
# requires-python = "==3.12.*"
# dependencies = [
#     "beat-this==1.1.0",
#     # torchaudio stopped at 2.11; a newer torch pairs it with a CUDA build.
#     "torch==2.11.0",
#     "torchaudio==2.11.0",
#     "soundfile",
#     "soxr",
# ]
#
# # CPU wheels: PyPI's default torch brings gigabytes of CUDA.
# [tool.uv.sources]
# torch = { index = "pytorch-cpu" }
# torchaudio = { index = "pytorch-cpu" }
#
# [[tool.uv.index]]
# name = "pytorch-cpu"
# url = "https://download.pytorch.org/whl/cpu"
# explicit = true
# ///
"""Reference beats from beat_this, in the Ballroom `.beats` format `eval` reads.

beat_this (Foscarin et al., ISMIR 2024) is the state of the art for beats
and downbeats. Its output is not ground truth, but on music no one has
annotated, agreeing with it says something and disagreeing says where to
listen.

    uv run tools/reference.py ~/Music/Hard\\ Tech data/reference/hardtech

Audio is decoded with ffmpeg rather than whatever soundfile manages, so
Opus and M4A work too. Files already done are skipped.
"""

import subprocess
import sys
from pathlib import Path

import numpy as np

AUDIO = {".mp3", ".flac", ".wav", ".aif", ".aiff", ".ogg", ".opus", ".m4a"}
RATE = 22_050


def decode(path: Path) -> np.ndarray:
    """Mono float32 at RATE, the way beat_this was trained."""
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1", "-ar", str(RATE), "-f", "f32le", "-"],
        check=True,
        capture_output=True,
    ).stdout
    return np.frombuffer(raw, dtype=np.float32)


def positions(beats: np.ndarray, downbeats: np.ndarray) -> list[int]:
    """Each beat's place in its bar, 1 on a downbeat. Beats before the first
    downbeat count back from it, so a pickup reads ... 3 4 | 1."""
    is_down = [bool(np.any(np.abs(downbeats - b) < 1e-6)) for b in beats]
    out, count = [], 0
    for down in is_down:
        count = 1 if down else count + 1
        out.append(count)
    if any(is_down):
        first = is_down.index(True)
        meter = max(out[first:]) if first < len(out) else 4
        for i in range(first):
            out[i] = meter - (first - 1 - i)
    return out


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit("usage: reference.py AUDIO_DIR OUT_DIR")
    source, target = Path(sys.argv[1]).expanduser(), Path(sys.argv[2])
    target.mkdir(parents=True, exist_ok=True)

    from beat_this.inference import Audio2Beats

    tracker = Audio2Beats(checkpoint_path="final0", device="cpu", dbn=False)
    files = sorted(p for p in source.rglob("*") if p.suffix.lower() in AUDIO)
    for n, path in enumerate(files, 1):
        out = target / f"{path.stem}.beats"
        if out.exists():
            continue
        try:
            beats, downbeats = tracker(decode(path), RATE)
        except Exception as err:  # one bad file should not stop a library
            print(f"{path}: {err}", file=sys.stderr)
            continue
        lines = [f"{b:.4f} {p}" for b, p in zip(beats, positions(beats, downbeats))]
        out.write_text("\n".join(lines) + "\n")
        print(f"[{n}/{len(files)}] {path.name}: {len(beats)} beats", flush=True)


if __name__ == "__main__":
    main()
