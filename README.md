# Sonorant

A real-time spectrogram, spectrum and loudness visualiser for Ubuntu and Windows. It
follows whatever player is running instead of living inside one.

Sonorant is the rewrite of [Nostalgia+](https://github.com/nifraz/NostalgiaPlus), a
MusicBee plugin, as a standalone Rust app drawn by its own WGSL shaders on wgpu. The
plan, with every decision and phase, is in [docs/plan.md](docs/plan.md).

## Status

Early work: Phases 0 to 2 are done and Phase 3, the renderer, has started. See
[Progress](docs/plan.md#progress) in the plan.

| Part | State |
|---|---|
| `sonorant-dsp` | Verified against Nostalgia+'s reference vectors: the multi-resolution FFT bank, all six windows, mid/side and single-channel modes, BS.1770 loudness and true peak, overs, dynamic range, curve shaping and ballistics, notes, tempo and brightness |
| `sonorant-core` | Every setting and preset, TOML settings, presets and themes, the Nostalgia+ importer, palettes, the analysis engine and thread, and a WAV source |
| `sonorant-platform` | WASAPI loopback of the whole system or one app on Windows; PipeWire on Linux (not yet compiled) |
| `sonorant-render` | The pane layout, the GPU history store and the spectrogram pass |
| `sonorant` | Captures, analyses and draws the live spectrogram with a status line and a provisional menu; `sonorant capture` runs the pipeline without a window |

## Building

Rust comes from [rustup](https://rustup.rs); `rust-toolchain.toml` pins the version.

**Ubuntu 24.04 or later**

```sh
sudo apt install pkg-config clang libclang-dev libpipewire-0.3-dev libspa-0.2-dev \
  libwayland-dev libxkbcommon-dev
cargo run --release
```

**Windows 10 or 11**

With the Visual Studio C++ build tools installed, `cargo run --release` is all there is.

The GNU toolchain (`x86_64-pc-windows-gnu`) works without Visual Studio, with three
adjustments to what rustup bundles:

- `windows-rs` needs a `dlltool` that doesn't depend on an assembler. rustup's
  `llvm-tools` component has one: copy its `llvm-ar.exe` as `llvm-dlltool.exe` and pass
  `-C dlltool=<path>` in `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS`.
- The bundled MinGW has no `libshlwapi.a`. Make one with
  `llvm-dlltool -d shlwapi.def -l libshlwapi.a -m i386:x86-64` from a `.def` file listing
  `AssocQueryStringW`, and put its folder in `LIBRARY_PATH`.
- The bundled GNU `ld` mis-merges LLVM's import libraries with MinGW's for the same DLL,
  and the program crashes before `main`. Link with LLD instead: set
  `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER` to a script that runs the bundled
  `x86_64-w64-mingw32-gcc` with `-B <dir>/` first, where `<dir>` holds a copy of
  `rust-lld.exe` named `ld.exe` and empty `crtbegin.o` and `crtend.o` objects, and
  with `-B` and `-L` pointing at the toolchain's `lib/self-contained`.

## Running

```sh
cargo run --release                        # capture everything the system plays
cargo run --release -- --app Spotify       # one app (a name or process id)
cargo run --release -- --wav song.wav      # a WAV file, looped, instead of capture
cargo run --release -- --pacing-seconds 30 --pacing-log pacing.csv
cargo run --release -- capture --seconds 10   # no window: print loudness once a second
cargo run --release -- apps                   # the apps that can be captured alone
```

Right-click for the provisional menu. **Space** or a click freezes the picture while
analysis carries on, and **F11** toggles fullscreen. The status line shows what is being
captured, loudness and tempo, the frame rate, the 99th percentile frame interval and the
refreshes missed. On exit the pacing figures for the whole run are logged.

Settings live in `%APPDATA%\Sonorant` or `~/.config/sonorant`. On the first run on
Windows, Nostalgia+'s settings, presets and themes are brought over from MusicBee.

## Tests

```sh
cargo test --workspace
```

The DSP tests compare against [tests/reference](tests/reference), numbers exported from
Nostalgia+ by its test harness (`build\export.cmd <dir>` in that repository). Spectra
have to agree within 0.01 dB above -120 dBFS, loudness within 0.01 LU, and notes, overs
and tempo exactly.

## Layout

```
crates/
  sonorant-dsp/       FFT bank, loudness and true peak, dynamic range, features
  sonorant-core/      settings, presets, palettes, menu model, analysis engine
  sonorant-platform/  PipeWire and MPRIS on Linux; WASAPI and SMTC on Windows
  sonorant-render/    wgpu passes and WGSL shaders
  sonorant/           the app: window, input, egui menus and dialogs
  sonorant-testdata/  loads the reference vectors for tests
tests/reference/      vectors exported from Nostalgia+
docs/plan.md          the rewrite plan
```
