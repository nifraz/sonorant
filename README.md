# Sonorant

A real-time spectrogram, spectrum and loudness visualiser for Ubuntu and Windows. It
follows whatever player is running instead of living inside one.

Sonorant is the rewrite of [Nostalgia+](https://github.com/nifraz/NostalgiaPlus), a
MusicBee plugin, as a standalone Rust app drawn by its own WGSL shaders on wgpu. The
plan, with every decision and phase, is in [docs/plan.md](docs/plan.md).

## Status

Early work. What exists today:

| Part | State |
|---|---|
| `sonorant-dsp` | Ported and verified against Nostalgia+'s reference vectors: the multi-resolution FFT bank, all six windows, mid/side and single-channel modes, BS.1770 loudness and true peak, overs, dynamic range, curve shaping and ballistics, notes, tempo and brightness |
| `sonorant-core` | Palettes, verified against the reference |
| `sonorant-render` | The scrolling spectrogram pass: a GPU ring of level rows, coloured through a palette and scrolled by audio time |
| `sonorant` | The Phase 0 skeleton: a window with wgpu and egui, a synthetic spectrogram, a test menu and frame-pacing measurement |
| `sonorant-platform` | Empty until Phase 2 (capture) |

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

## Running the skeleton

```sh
cargo run --release -- --help
cargo run --release -- --pacing-seconds 30 --pacing-log pacing-60hz.csv
```

Right-click for the test menu. **Space** pauses the scroll, **F11** toggles fullscreen.
The status line shows frame rate, the 50th and 99th percentile frame interval and the
refreshes missed, measured from the moments frames get their swapchain image. On exit
the same figures are logged for the whole run.

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
