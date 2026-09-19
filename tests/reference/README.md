# Reference vectors

Numbers produced by Nostalgia+ for inputs chosen to exercise every part of its DSP and
model. Sonorant's tests run the same inputs through the Rust code and compare.

They were written by Nostalgia+'s test harness, from the commit named in
`manifest.json`:

```
build\export.cmd <output dir>
```

Nothing here is edited by hand. To regenerate, run that command in the Nostalgia+
repository and copy the output over this folder.

## Files

| File | What it holds |
|---|---|
| `manifest.json` | The generating commit and the section list |
| `signals/*.f64` | The analysed audio: interleaved little-endian `f64`, left then right. Every value is exactly an `f32`, as the capture ring held it |
| `fft.json` | Window coefficients and coherent gains, raw transforms, single and paired magnitude spectra |
| `frequency_map.json` | Band edges and centres for each axis, and position probes |
| `notes.json` | Note names and cents across 10 Hz to 24 kHz, including exact midpoints between semitones |
| `spectrum.json` | Spectra for every profile, window, axis, aggregate, tilt, pair mode and channel mode, at 48 and 44.1 kHz |
| `dynamic_range.json` | Floor and ceiling after every update, for ramps, silence and steps |
| `curve_shaping.json` | Every interpolation and filtering amount, and the extremum traces over time |
| `loudness.json` | K-weighting filters per rate, the true-peak interpolator, and every reading after every chunk for calibration tones, a 10 dB swing, bursts, sustained clipping and music-like signals |
| `music_features.json` | Flux, onset, pulse, tempo and centroid per frame for clicks, steady tones, bass and treble, and an uneven clock |
| `pipeline.json` | The whole per-frame analysis as the views ran it, for three settings |
| `palettes.json` | Every palette's 256 entries, plain and hue-shifted |
| `settings.json`, `settings/` | Settings files as Nostalgia+ wrote them (default, each preset, every field changed, an old file with renamed keys and bad values, a theme) and the values each loads back as |
| `timeline.json` | Rows per second for each frame rate, scroll divider and cinematic mode |
| `layout.json` | Centre deck and pane rectangles. Informative: Sonorant uses another font |
| `screenshots/` | Nostalgia+'s screenshots, the parity reference for the renderer |

Short or analytic inputs, such as sines, are described in the JSON by the formula that
made them rather than stored, and the loudness cases carry the sum and energy of their
input so a generator mismatch can be told from a meter mismatch.

## Tolerances

Spectra agree within 0.01 dB wherever the reference is above -120 dBFS, loudness within
0.01 LU, and notes, overs and tempo exactly. Plain arithmetic on the same inputs, such as
windows, maps and palettes, is held to rounding error or exact equality.
