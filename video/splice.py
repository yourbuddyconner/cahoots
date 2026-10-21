"""Splices the launch track: the intro and first main section, then the outro tail.
Cuts on detected beats with a 30 ms equal-power crossfade, fades the tail. Writes out/music.wav."""
import json, sys
import numpy as np, soundfile as sf
src, beats_path, out = sys.argv[1:4]
y, sr = sf.read(src, dtype="float32")
beats = json.load(open(beats_path))
def nearest(t): return min(beats, key=lambda b: abs(b - t))
cutA = nearest(41.5)          # end of segment A, on a beat
cutB = nearest(137.0)         # start of segment B, on a beat
xf = int(0.03 * sr)
a = y[: int(cutA * sr) + xf]
b = y[int(cutB * sr) - xf :]
w = np.linspace(0, np.pi / 2, 2 * xf)[:, None]
a[-2 * xf :] *= np.cos(w)
b[: 2 * xf] *= np.sin(w)
mix = np.concatenate([a[: -2 * xf], a[-2 * xf :] + b[: 2 * xf], b[2 * xf :]])
tail = int(2.0 * sr)
mix[-tail:] *= np.linspace(1, 0, tail)[:, None]
sf.write(out, mix, sr, subtype="PCM_24")
print(f"cutA {cutA:.3f}s cutB {cutB:.3f}s  length {len(mix)/sr:.2f}s")
