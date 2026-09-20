# Sonorant

A real-time spectrogram, spectrum and loudness visualiser for Ubuntu and Windows. It
follows whatever player is running instead of living inside one.

Sonorant is the rewrite of [Nostalgia+](https://github.com/nifraz/NostalgiaPlus), a
MusicBee plugin, as a standalone Rust app drawn by its own WGSL shaders on wgpu. The
plan, with every decision and phase, is in [docs/plan.md](docs/plan.md).

## Status

Early work: Phases 0 to 3 are done, so the renderer draws the parity views; Phase 4,
now playing, is next. See
[Progress](docs/plan.md#progress) in the plan.

| Part | State |
|---|---|
| `sonorant-dsp` | Verified against Nostalgia+'s reference vectors: the multi-resolution FFT bank, all six windows, mid/side and single-channel modes, BS.1770 loudness and true peak, overs, dynamic range, curve shaping and ballistics, notes, tempo and brightness |
| `sonorant-core` | Every setting and preset, TOML settings, presets and themes, the Nostalgia+ importer, palettes, the analysis engine and thread, and a WAV source |
| `sonorant-platform` | Windows: WASAPI loopback of the whole system or one app, and now playing from SMTC. Linux: PipeWire capture and now playing from MPRIS, neither yet run on a Linux machine |
| `sonorant-render` | The whole picture: the pane and deck layouts, the GPU history store, the spectrogram, the curve strips, a text and shape overlay (IBM Plex, bundled) carrying the grid, scales and labels, the waveform lanes, goniometer, meters and readouts, the colour bar and status line, the floating-point target and its glow, GPU pass timing, and golden renders |
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
cargo run --release -- --screenshot shot.png  # save the picture after 5 s, then exit
cargo run --release -- --settings some/folder # settings of its own, for clean runs
cargo run --release -- capture --seconds 10   # no window: print loudness once a second
cargo run --release -- apps                   # the apps that can be captured alone
```

Right-click for the menu: presets, what is captured, and every setting, each with its
key alongside it and a line saying what it does. **F1** opens the same list as a
searchable window, and clicking an entry there does it.

| Key | |
|---|---|
| `Space` | Freeze the picture; analysis carries on |
| `F11`, `Esc` | Fullscreen, and back |
| `I` | Immersive mode: the glow, the beat flare, a fading chrome and the palette drifting with the music's brightness |
| `A` | Hold the average spectrum in amber to compare against, or drop it |
| `O` | The on-screen readouts: the hover box, the quick bar and the status line |
| `G` | The frequency grid |
| `W` | The waveform lanes |
| `P` | The peak trace |
| `B`, `C` | The next curve style, the next palette |
| `F1` | Help |

Point at a pane to read out the frequency under the pointer, as hertz and as a note with
its deviation in cents, each channel's level there, and how far back in time the column
is. `Hover` in the menu adds ghost lines at the harmonics of that frequency and stamps
the reading onto the axis. Double-clicking a column of the image sends the player to that
moment, and the deck's transport buttons and seek bar work on whatever player is being
followed. The strip of buttons over the image is the quick bar, for the switches reached
most often; it can be made compact or switched off.

The status line shows what is being captured, loudness and tempo, the frame rate, the
99th percentile frame interval and the refreshes missed. On exit the pacing figures for
the whole run are logged.

Settings live in `%APPDATA%\Sonorant` or `~/.config/sonorant`, and are saved on exit.
Presets you save go beside them, and the menu loads and deletes them. On the first run on
Windows, Nostalgia+'s settings, presets and themes are brought over from MusicBee. The
desktop's accent colour and its dark or light preference stand in for the skin colours
the MusicBee plugin took from its host.

## Tests

```sh
cargo test --workspace
```

The DSP tests compare against [tests/reference](tests/reference), numbers exported from
Nostalgia+ by its test harness (`build\export.cmd <dir>` in that repository). Spectra
have to agree within 0.01 dB above -120 dBFS, loudness within 0.01 LU, and notes, overs
and tempo exactly.

The Linux code can at least be type-checked from Windows, which is worth doing before
sending anything to CI. PipeWire needs its development headers, so capture is behind a
feature that this leaves off; everything else, MPRIS included, is compiled:

```sh
rustup target add x86_64-unknown-linux-gnu
cargo check -p sonorant-platform --target x86_64-unknown-linux-gnu --no-default-features
```

## Layout

```
crates/
  sonorant-dsp/       FFT bank, loudness and true peak, dynamic range, features
  sonorant-core/      settings, presets, palettes, menu model, analysis engine
  sonorant-platform/  PipeWire, MPRIS and the portals on Linux; WASAPI, SMTC
                      and UISettings on Windows
  sonorant-render/    wgpu passes and WGSL shaders
  sonorant/           the app: window, input, egui menus and dialogs
  sonorant-testdata/  loads the reference vectors for tests
tests/reference/      vectors exported from Nostalgia+
docs/plan.md          the rewrite plan
```
