# Sonorant: rewrite plan

*2026-09-19. Replaces the earlier .NET port plan (Avalonia + SkiaSharp). Every decision
is now made; the last twelve were settled the same day.* Sonorant
is Nostalgia+ rewritten from scratch in Rust as a standalone app for Ubuntu and Windows. It
keeps every feature of the MusicBee plugin, adds four visuals that only a GPU renderer makes
possible, and follows whatever player is running instead of living inside one.

## Summary

- **New code in a new `sonorant` repository.** The [Nostalgia+ repo](https://github.com/nifraz/NostalgiaPlus) is frozen as the plugin's final
  home. It also serves as the specification: its test harness exports reference numbers
  that the Rust DSP must reproduce.
- **Rust throughout.** Drawing is done by our own WGSL shaders on wgpu, which runs on Vulkan
  on Ubuntu and Direct3D 12 on Windows. Menus, the help window and dialogs use egui and are
  drawn in the same frame as the visuals.
- **Three threads that never wait on each other:** capture (real-time), analysis (a fixed
  hop in audio time) and render (at the display's refresh rate). Data only flows forward,
  through lock-free queues.
- **Smooth at any refresh rate.** The screen updates at the monitor's rate, from 60 to
  240 Hz, while the spectrogram scrolls by audio time. It moves at the same speed on every
  display, just more smoothly on faster ones.
- **Ubuntu:** native PipeWire capture, of the whole mix or just the player's stream, and
  now-playing from MPRIS. **Windows:** WASAPI loopback, system-wide or per process, and
  now-playing from SMTC.
- **v1 is full parity plus four new visuals:** a phosphor scope with bloom, a zoomable long
  history, a 3D waterfall and a beat-reactive backdrop.

## Decisions

| Decision | Choice | Status |
|---|---|---|
| Name | **Sonorant**: binary and package `sonorant`, Flatpak ID `io.github.nifraz.Sonorant` | Decided |
| Player integration | Standalone app. MPRIS on Ubuntu, SMTC on Windows | Decided |
| Windows plugin | Retired. The standalone app replaces it on Windows too | Decided |
| Approach | Rewrite from scratch. The C# code is the reference, not a source to port | Decided |
| Language | Rust, stable toolchain, edition 2024 | Decided |
| Graphics | wgpu with our own WGSL shaders: Vulkan on Ubuntu, Direct3D 12 on Windows | Decided |
| UI layer | egui, themed to match, with AccessKit for screen readers | Decided |
| Window and input | winit, which follows from wgpu and egui | Decided |
| Linux capture | Native PipeWire from day one | Decided |
| Frame timing | Display refresh rate. Scrolling follows audio time | Decided |
| v1 scope | Parity, plus phosphor scope and bloom, zoomable long history, 3D waterfall and beat-reactive backdrop | Decided |
| Repository | New [`sonorant`](https://github.com/nifraz/sonorant) repo. The Nostalgia+ repo stays as the plugin's home | Decided |
| Licence | GPL-3.0-or-later | Decided |
| Text stack | cosmic-text and glyphon for the deck and axis labels; egui's own text for menus and help | Decided |
| Font | IBM Plex Sans, with IBM Plex Mono for readouts (SIL Open Font License) | Decided |
| Targets | The [targets](#targets) table as written, on two reference machines (below) | Decided |
| Settings format | TOML, with a one-time importer for Nostalgia+'s `key=value` files | Decided |
| History | 5 minutes at 60 rows per second, each row keeping its own range; a "global range" setting; length 1 to 15 minutes | Decided |
| Visual delay | A manual offset, filled in automatically where PipeWire reports the sink's latency. Phase 7 | Decided |
| Ubuntu | 24.04 LTS and 26.04 LTS, x64 and arm64 | Decided |
| Windows | Windows 10 22H2 and Windows 11, x64. arm64 later | Decided |
| Packaging | Flatpak and `.deb`; a zip and winget; the Microsoft Store later | Decided |
| Code signing | SignPath Foundation | Decided |
| Store title | "Sonorant – Music Visualizer" | Decided |

## Progress

*As of 2026-09-20.* Phases 0 to 5 are written, apart from the checks that need other
machines, a real player or CI. Every piece Phase 3 deferred has now arrived: the
backdrop in Phase 4, and the harmonic ruler and the quick bar here. The three targets
missed at the start are met or explained (see [Measurements](#measurements)). Phase 6
begins at [Next](#next).

**Phase 0 (repository, CI, skeleton): done, except clean frame pacing and CI**

- [x] The `sonorant` repo, with the plan as its first commit. Five crates plus
  `sonorant-testdata`, Rust 1.98.1 pinned, edition 2024, GPL-3.0-or-later.
- [x] The CI workflow: fmt, clippy with warnings as errors, tests on `ubuntu-24.04`,
  `ubuntu-24.04-arm` and `windows-latest`, and cargo-deny. Clean locally; it first runs
  on the push to GitHub.
- [x] The skeleton (winit, wgpu and egui, a test menu, frame-pacing measurement with
  `--pacing-seconds` and `--pacing-log`). It has since grown into the live app below.
- [x] Reference vectors: `build\export.cmd` in Nostalgia+ (branch
  `sonorant-reference-export`, not yet merged) wrote `tests/reference/`, and the
  screenshots are copied beside them.
- [x] Frame pacing measured as the viewer sees it: frames delivered against the
  refreshes in the same time, at the compositor's exact rate (59.94 Hz here, which winit
  rounds to 59). The old per-interval count read 8% missed where every refresh got a
  frame; the swapchain just hands buffers back in bursts. On the Windows 10 reference PC
  a 40 s windowed run misses 0.5% of refreshes with 40-50% background load. DXGI's own
  present counters are read where they move (independent flip); under Optimus
  composition they don't. Stalls over 100 ms are logged with where the time went.
- [x] No dropped frames in steady state, fullscreen: a 12 s run at 1920x1080 with the
  glow on delivered 687 frames and missed none. Windowed it's 0.5%, where the window is
  composited with everything else on a busy machine.
- [ ] The same on Ubuntu (GNOME Wayland at 125% and 150%), on Windows 11, and on a
  144 Hz monitor: not measured yet.
- [ ] CI green on all three runners.

**Phase 1 (DSP, verified): done**

- [x] `sonorant-dsp`: the FFT bank in all four profiles, paired stereo transforms, six
  windows, tilt, auto-range, BS.1770 loudness with true peak, crest and overs, tempo,
  brightness, curve shaping and ballistics, the frequency map and notes. Nine reference
  suites pass within the plan's tolerances, at 48 and 44.1 kHz, with filters checked at
  16, 32, 88.2 and 96 kHz as well.
- [x] The analysis engine: a fixed hop in audio time, so output doesn't depend on
  chunking. At 60 hops per second it reproduces Nostalgia+'s whole per-frame pipeline
  (curves, extremes, range, features). The analysis thread publishes through rtrb and a
  triple buffer.
- [x] Settings persistence, user presets, themes and the image timeline, ported as tests.
  The TOML store and the Nostalgia+ importer have been checked against the exported files
  and against a real Nostalgia+ install.
- [x] Benchmarks with criterion, and the hop-time target: a Balanced hop now takes a
  median 0.37 ms against 0.5 ms (from 1.18 ms). Projection plans per map, power spectra
  with one root per column, a table-driven `log10` in `sonorant-dsp::math` in place of
  the C runtime's (30 to 50 ns a call under MinGW), windowing straight from the capture
  history, and the 8K-and-up transforms refreshing at most 60 times a second of audio.
  Every reference suite still passes; at 60 hops a second every band still refreshes
  every hop. `examples/hop_times.rs` in `sonorant-core` prints the per-hop distribution.
- Moved: the "menu actions" tests go with the menu model in Phase 5, and the centre deck
  and quick bar "layout budgets" go with that layout in Phase 3. The pane layout is
  already checked against the reference, rectangle for rectangle.

**Phase 2 (capture): done on Windows; Linux and live checks pending**

- [x] The `AudioSource` trait with its status states, and a WAV file source.
- [x] Windows: event-driven WASAPI loopback on an MMCSS thread, process loopback, default
  device changes (polled once a second), the exclusive-mode state, silence fed while
  nothing plays, and a list of apps with audio sessions. Whole-system capture runs on the
  reference PC at 48 kHz with nothing dropped.
- [x] Linux: a PipeWire stream on the default sink's monitor or on one app's node, asking
  for a 256-frame quantum, plus an app list from the registry. It's written against
  pipewire-rs 0.10 but has never been compiled; the first Linux CI run will be its first
  build. Its process callback runs on the source's own loop thread rather than PipeWire's
  real-time one, so the silence timer and capture share the ring's single producer.
- [ ] Process loopback and device switching, exercised with sound playing.
- [ ] Capture-to-analysis latency measured (target under 15 ms).
- [ ] Ubuntu 24.04 and 26.04, and Windows 11.

**Phase 3 (renderer, parity): done, apart from three pieces that belong to later phases**

- [x] The pane layout, the history store (Float16 level pairs in a texture array, each
  row's range in a side texture, row timestamps on the CPU), and the spectrogram pass. It
  maps any axis, range and palette per pixel, scrolls by the audio clock, and freezes.
- [x] Start-up: only the platform's backend opens (Direct3D 12, Vulkan on Linux), the
  rest only as a fallback, and capture opens in parallel. 4.3 s became 1.1-2.7 s; what's
  left is waking the GeForce behind Optimus (see [Measurements](#measurements)).
- [x] **Curves** (`curves.rs`): line with its gradient fill, bars and LED, the peak,
  average, minimum and amber reference traces (`A` holds and drops it, `B` cycles the
  style), and the plain, lines, grid and chessboard backgrounds. One fragment pass per
  strip measures each pixel's distance to the segments near it, so lines are
  anti-aliased and a translucent trace doesn't bead where its segments meet.
- [x] **Text and scales** (`overlay.rs`, `axes.rs`): rectangles, gradients,
  anti-aliased lines and text in three layers around the curves, with glyphon and
  cosmic-text drawing IBM Plex Sans (Regular, Light, Medium) and Plex Mono, bundled
  under the OFL and shaped once per label. The frequency grid and its labels (octaves
  down to semitones as the space allows, as notes, hertz or both), the gutter and outer
  label columns with their unit captions, semitone lines, time marks, the level scale,
  the scale lane and the channel labels.
- [x] **The bottom band** (`band.rs`, `deck.rs`): the waveform lanes and the centre
  deck, ported rule for rule. The 36 reference deck layouts in `tests/reference/layout.json`
  match rectangle for rectangle. Drawn: the goniometer, the transport with its shared
  bar column, correlation and balance, the readout grid, the colour bar, and the status
  line over the image, where Nostalgia+ had it (egui no longer draws one).
- [x] **The glow and the immersive treatment** (`bloom.rs`): the visuals render into a
  linear-light floating-point target; the glow is thresholded and blurred at an eighth
  of the size and composited back, as the GDI+ version did. `I` turns immersive mode on,
  with the beat flare and the palette drifting with the music's brightness.
- [x] **Golden renders** (`tests/golden.rs`): the whole scene drawn offscreen and
  compared with a committed picture, one per software renderer (WARP's is committed;
  lavapipe's will be on the first Linux run). On a hardware GPU it checks the frame is
  drawn rather than its pixels. It found a real bug immediately: the overlay's
  screen-size uniform read as zeros on WARP, which put every shape at infinity.
- [x] **GPU timing** (`timing.rs`): timestamp queries around each pass, with the total
  in the status line. At 1920x1080 on the reference PC a frame costs 2.9 ms without the
  glow and 3.3 ms with it (see [Measurements](#measurements)).
- [x] The 2D layers blend sRGB-encoded colours, as GDI+ did, drawing on the swapchain's
  plain view; blended in linear light, Nostalgia+'s faint lines would come out several
  times brighter. The spectrogram and the glow work in linear light.
- [x] `--screenshot <file.png>` saves the window's own frame, `--settings <dir>` runs
  with a settings folder of its own, and `sonorant-core`'s `hop_times` example prints
  the per-hop distribution. All three are for checking work without a screen capture.

Three pieces were written but could only be finished later, and moved:

- **The backdrop** needed album art, and arrived with it in Phase 4.
- **The harmonic ruler** is drawn from the hover position, so it moves to Phase 5 with
  the hover readout.
- **The quick bar** is a strip of buttons that do nothing until the menu model and input
  exist, so it moves to Phase 5 with them. Its `Reserve`/`HeightFor` tests go with it.

Parity is judged against Nostalgia+'s current code and its exported layouts, not against
the screenshots in `tests/reference/screenshots/`: those were taken before the centre
deck was rebuilt, and still show the title and loudness figures floating in the screen's
top corners.

**Phase 4 (now playing): written on both platforms; not yet seen following a player**

- [x] **The `MediaSession` trait and the now-playing types** (`sonorant-core::media`):
  the followed player, its metadata, artwork, play state, capabilities and transport
  commands, published as a snapshot the frame loop copies out of a mutex. Nothing in
  the frame ever waits on WinRT or D-Bus.
- [x] **The position clock.** Neither platform reports the position continuously, so a
  reading is kept with the instant it was taken and carried forward from there,
  resyncing about once a second while playing. It holds where it was on a pause, never
  runs past the end, and is unit-tested.
- [x] **SMTC** (`windows/smtc.rs`) on its own thread, driven by the session manager's
  and the session's change events with a one-second fallback tick. Thumbnails are read
  once a track, and `LastUpdatedTime` says how old a position reading already was, so
  the seek bar is right rather than a second fast. **Two fields stay empty on Windows:
  SMTC carries no composer and no year.**
- [x] **MPRIS** (`linux/mpris.rs`) over zbus: every deck field from `xesam:*`, the year
  from `contentCreated`, the length from `mpris:length`, artwork from `mpris:artUrl`,
  and the player's process id from `GetConnectionUnixProcessID`. Players are polled
  every 400 ms with one `GetAll` each rather than subscribed to; `PropertiesChanged`
  and `Seeked` are the refinement, and cost up to 400 ms on a track change until then.
- [x] **Artwork** (`render/artwork.rs`): PNG and JPEG decoded off the frame loop, fitted
  to 512 pixels, and uploaded as two pictures. The deck draws the cover stretched into
  its square, between the overlay's layers so the frame lands on top of it as GDI+
  painted it; the backdrop draws a 40-pixel copy over the whole window, into the visuals
  target before the spectrogram, so it is ground the analysis covers. That is Phase 3's
  last deferred piece.
- [x] **Track-change resets:** a new track id starts LUFS-I, LRA, BPM and overs again.
  Players that report no id fall back to the title and album, which catches a change
  without throwing away a minute of integrated loudness on a metadata refresh.
- [x] **Capture follows the player.** The analysis thread can be handed a new ring, so
  the source is swapped without restarting analysis or interleaving two streams. Capture
  moves to the followed player's process when there is one and back to the whole mix
  only when that player has gone or its source has stopped: a pause is not a reason to
  move, because every switch costs the audio clock its bearings. A Capture menu and a
  "Follow player" submenu choose between them, and the status line says what it
  followed. A `--wav` or `--app` run is left alone.
- [x] The Linux code compiles for the first time. PipeWire needs its development
  headers, so capture moved behind a default feature; with it off, `cargo check --target
  x86_64-unknown-linux-gnu --no-default-features` builds and lints the crate from
  Windows. MPRIS is checked and linted this way. PipeWire capture still is not.
- [ ] **Run against real players.** Nothing here has been seen working: Windows needs
  MusicBee, Spotify and a browser; Ubuntu needs Rhythmbox, Strawberry, Spotify, Firefox
  and VLC, and a Linux machine. `cargo run -p sonorant-platform --example now_playing`
  prints what the session sees, on either platform, without the app.
- [ ] **`https://` artwork.** Spotify and the browsers report web URLs over MPRIS.
  Reaching them needs an HTTP client and a TLS stack, which is a download-size decision
  held until the size pass; the `ArtFetcher` seam is there and the loader logs what it
  skipped. Until then those players show the empty frame on Ubuntu. On Windows it does
  not arise: SMTC hands over the thumbnail itself.

**Phase 5 (app shell): written; the scaling and parity checks need other machines**

- [x] **The menu is a model** (`sonorant-core::menu`). `MenuFactory.cs` built the menu in
  code, and Nostalgia+'s keyboard handler and help text held the same knowledge
  separately, so the three drifted. Here one tree of items carries each item's label, its
  state, its shortcut, its help line and the action it performs; `tree` builds it from
  the current state, and the menu, the keys and the help search all read it. Tests assert
  that no key is bound twice, that every command explains itself, and that a key does
  what its own menu item does.
- [x] **Actions name a setting** rather than reaching for a field, so a switch, a number
  or a choice is one value the model can apply, mark as current and search. `Flag` covers
  all 50 switches and `Number` the 21 numbers, each with the range it is held inside; a
  test flips every flag and shows it moves nothing else. What the model cannot do itself,
  because it needs the settings folder, comes back as an `Effect`.
- [x] **Keys:** `Space`, `F11`, `Esc`, `I`, `A`, `O`, `G`, `W`, `P`, `B`, `C` and `F1`.
  The plugin's bindings weren't in the exported reference, so the eight the checklist
  names were mapped to what they most plainly stand for, and `B` (the next curve style,
  which the skeleton already had), `C` (the next palette) and `F1` (help) were added
  beside them. **A click no longer freezes:** a double-click over the image seeks, and
  two meanings for one button on the same pixels is worse than one key.
- [x] **The hover readout** (`render/hover.rs`): the frequency under the pointer in
  hertz and as a note with its deviation in cents, each channel's level at that
  frequency, and how far back in time the column is. `sync_hover` reads out both panes,
  `show_hover_pin` stamps the reading onto all three label columns, and `show_harmonics`
  draws the **harmonic ruler** — Phase 3's second deferred piece — as ghost lines at
  whole multiples of the hovered frequency, each fainter than the last.
- [x] **The quick bar** (`render/quickbar.rs`), Phase 3's last deferred piece. `Reserve`
  and `HeightFor` carry over with their tests: the strip is taken off the top of the view
  before the panes are laid out, rather than drawn over the image, so nothing it covers
  is analysis and a button is never also a row of the image. Its buttons are `Action`s
  from the menu model, so a button and its menu item cannot come to mean different
  things. Split around the gutter, the axis runs from the top of the window unbroken.
- [x] **The help window and the name dialog.** Help is the menu tree flattened, searched
  by path, help line or key, and clicking an entry performs it. The name dialog saves the
  settings as a preset of your own; the button says "Replace" when the name is taken.
- [x] **Presets:** the eight built-in ones and the user's own, loaded, saved and deleted
  from the menu. Changing anything marks the settings `Custom`, so the list stops
  claiming a preset the settings have moved away from.
- [x] **Immersive mode's fade** (`chrome.rs`): with `auto_hide`, the scales, the deck,
  the quick bar, the status line and the readout fade out after 2.5 s of nothing
  happening, and the pointer goes with them. A moved pointer, a key or an open menu
  brings them back at once rather than fading in. The layout doesn't move when they go:
  relaying out would make the history jump, which is worse than the pixels are worth.
- [x] **The transport works.** The deck's buttons and its seek bar reach
  `NowPlaying::send`, which still refuses a control the player says it cannot do, and a
  double-click on a spectrogram column asks the player for that moment.
- [x] **The OS accent colour and dark or light** replace MusicBee's skin colours:
  `UISettings` on Windows, the XDG settings portal on Linux. The accent fills in the
  hover slot when no colour was chosen for it, without being written into the saved
  theme, so it follows the desktop instead of freezing at whatever it was on the day.
  Dark or light picks egui's theme.
- [x] **HiDPI.** The panes stay in physical pixels, so the image keeps a row to a pixel
  and loses no detail at 150%; every fixed size around them — the deck, the gutter, the
  label columns, the scale strip, the bars and the quick bar's air — is scaled to the
  display. `ScopeLayout::new`, `BandLayout` and `DeckLayout::new` take the scale, and the
  36 reference deck layouts pass it 1.0 and still match rectangle for rectangle.
- [x] **Keep the screen awake** while fullscreen and playing:
  `SetThreadExecutionState` on Windows, the inhibit portal on Linux.
- [x] **The frame-rate cap** works: `TargetFps` became display, 60 or 30, and the loop
  sleeps to it rather than spinning. Scrolling follows audio time whatever it is, so the
  cap changes how often the screen is redrawn and nothing about the picture.
- [ ] **The parity checklist**, on GNOME Wayland at 100%, 125% and 200% and on Windows 10
  and 11. Only 100% on Windows 10 has been seen. The scaled layout is written and its
  arithmetic is tested, but no one has looked at it on a scaled screen.
- [ ] **The transport and the seek bar against a real player.** They are wired to the
  same `send` Phase 4 left guarded, and nothing has pressed them with a player running.

### Next

Phase 6, the new visuals: the phosphor scope, the zoomable long history, the 3D
waterfall, the beat-reactive backdrop and the quality setting. The history store already
holds more than the screen shows and the spectrogram pass already maps any axis and range
per pixel, so zooming is mostly a matter of what the panes ask it for; `px_per_row` and
the hover's age-at-a-column arithmetic are the beginnings of it.

**A flaky golden, found here and not caused here.** `the_scene_renders_as_it_did` fails
about one run in three on WARP, always on the same single column of pixels: x=477, where
the right pane's curve strip begins and its viewport's left edge falls. The same test
binary passes and fails across runs with the layout identical every time (gutter
443..477, the strip's base column at 477), so it is a rasterisation edge case at the
viewport boundary rather than a change in what is drawn. It is older than the work that
found it: the test was run at HEAD to check. Worth fixing before CI, where a
one-in-three flake is worse than here: either draw the strip without a viewport, using a
scissor or the rect in the shader, or widen the golden's tolerance for a single boundary
column. Not fixed here because it is Phase 3's renderer and wants a careful look rather
than a guess.

Before that, or alongside it, the things that need a machine this one isn't: the
first CI run, which is the first PipeWire build and the first lavapipe golden; then
the player checks in Phase 4 and the scaling checks above. Carried over: the first
frame's 100-odd ms of lazy initialisation (logged as a stall) could move into start-up,
and swapping the capture source happens on the frame loop's thread, so following
a player costs a frame. Two new ones: the settings are only written on exit, so a
crash loses what the menu changed, and the history is sized for five minutes at the
scroll speed it started with, so changing the speed changes how far back it reaches.

### Measurements

| What | At the start | Now | Target |
|---|---|---|---|
| Whole hop at 120 hops per second, Balanced (engine, per hop) | 1.18 ms | **Met:** median 0.37 ms, mean 0.48 ms, minimum 0.20 ms | Under 0.5 ms |
| Display projection, 1,080 columns / history grid, 2,048 bins | 0.32 / 0.59 ms | 0.03-0.07 / 0.06-0.14 ms | |
| Frame pacing, 59.94 Hz panel, GeForce 840M on Direct3D 12, windowed, 40 s | 8% "missed" (counted per interval), stalls of 0.5 s | 59.6 fps, 11 of 2,205 refreshes missed (0.5%); one 0.5 s stall in five runs, blocked in the swapchain | No dropped frames in steady state |
| First frame, release build | 4.3 s | 1.1-2.7 s, of which opening the GPU is 0.8-1.9 s | Under 300 ms |
| GPU time a frame, 1920x1080 fullscreen, GeForce 840M | not measured | 2.9 ms (visuals 1.0, composite 0.8, furniture 1.0); 3.3 ms with the glow | Under 3 ms at 2560x1440, under 6 ms with every new visual |
| Release binary (Windows, GNU toolchain) | 14.2 MB | 16.9 MB; 7.1 MB zipped | Under 15 MB download |

The GPU figures are from the 840M at 1920x1080, which is what this machine's screen
allows; 2560x1440 has to be timed offscreen, which is a Phase 7 job. A fullscreen run
with the glow on held 59.9 fps over 687 frames with no missed refreshes.

The CPU is the i7-4510U, a 2014 laptop part, and these figures were taken on battery
with 40-50% of the CPU busy elsewhere, so the spread is wide. Start-up's floor
here is Direct3D 12 device creation waking the GeForce 840M from Optimus power-off;
everything else takes about 0.3 s. The Intel HD 4400 is always on but reachable only
through OpenGL, where pipeline creation fails on a downlevel limit (a Phase 7 item),
so 300 ms on this machine would need the window up before the GPU is. The pacing
figure is frames delivered against refreshes elapsed; with frames queued ahead,
the swapchain's buffers come back at 15.5 and 31 ms intervals that average one refresh.

### Building on the Windows reference PC

The PC has no Visual Studio, so it builds with Rust's GNU toolchain. That needs three
adjustments, all described in the README: an assembler-free `dlltool`, a `libshlwapi.a`,
and linking with LLD, because the bundled GNU `ld` mis-merges import libraries and the
program crashes before `main`. `criterion` stays at 0.5 because later versions compile C.
CI builds with MSVC and isn't affected.

## Targets

These are the numbers that "smooth, fast and real time" mean. Each phase gates on the ones
it affects.

**Reference machines:**

- **Windows:** the development PC. Core i7-4510U, 8 GB, Intel HD 4400 plus a GeForce 840M
  (2 GB), 1920×1080 at 60 Hz, Windows 10 22H2. It is the low-end floor and is measured on
  the 840M: Intel no longer ships Direct3D 12 for Haswell graphics and never shipped Vulkan
  for it on Windows, so wgpu can only reach the HD 4400 through OpenGL.
- **Ubuntu:** a recent laptop with Intel Iris Xe graphics or newer, which the
  "integrated GPU" figures below are written for.
- The 2560×1440 figures are timed offscreen, and a 144 Hz monitor is borrowed for the
  frame-pacing checks.

| Measure | Nostalgia+ today | Sonorant target |
|---|---|---|
| Frame rate | 60 fps from a sleep loop with `timeBeginPeriod` | Display refresh (60–240 Hz) on vsync, with no dropped frames in steady state |
| Scrolling | One row per frame | By audio time, in fractional rows. Same speed on every display |
| Analysis | 1.4–2.5 ms per frame | Under 0.5 ms per hop (Balanced profile, stereo, 48 kHz) |
| GPU time per frame | None (GDI+ draws on the CPU) | Under 3 ms at 2560×1440 on an integrated GPU for the parity views, and under 6 ms with every new visual on |
| Audio to screen | Capture buffer plus up to one frame | Under 30 ms from the mix to the photon, not counting the display's own lag |
| CPU | Not measured | Under 5% of one core at 144 Hz in the default view |
| Memory | Not measured | Under 300 MB with 5 minutes of history |
| Start-up | Loads inside MusicBee | First frame in under 300 ms |
| Download | 61 KB DLL | Under 15 MB |

## Starting point

Nostalgia+ is about 12.2k lines of C# built with the .NET Framework 4.0 `csc.exe`, with no
dependencies. Nothing is carried over as code. Each part is used as a specification:

| Nostalgia+ | Lines | Sonorant | What carries over |
|---|---|---|---|
| [src/Dsp/](https://github.com/nifraz/NostalgiaPlus/tree/main/src/Dsp) | ~1,700 | `sonorant-dsp` | The algorithms and their numbers, verified against exported reference vectors. Constants that assume 60 fps become seconds |
| [Settings.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/src/Settings.cs) | 908 | `sonorant-core` | Every setting and preset. A new file format with an importer for the old one |
| [PlayerBridge.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/src/PlayerBridge.cs) | 58 | `MediaSession` trait | The fields the deck uses. Redesigned around a cached, extrapolated position |
| [src/Render/](https://github.com/nifraz/NostalgiaPlus/tree/main/src/Render), `CenterDeck`, `BottomBand`, `QuickBar`, `Immersion` | ~3,800 | `sonorant-render` | The look and the layout rules, together with their tests. The drawing itself is redesigned for the GPU |
| `AnalyzerPanel`, `FullscreenView`, `MenuFactory`, `HelpWindow`, `NameDialog` | ~3,200 | `sonorant` (the app) | Behaviour, keys and menu contents. `MenuFactory` becomes a menu model |
| [src/Audio/](https://github.com/nifraz/NostalgiaPlus/tree/main/src/Audio) | ~300 | `sonorant-platform` | WASAPI knowledge. Rewritten on windows-rs. PipeWire is new |
| [Plugin.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/src/Plugin.cs), [MusicBeeInterface.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/src/MusicBeeInterface.cs) | ~620 | None | Dropped |
| [TestHarness.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/build/TestHarness.cs) | 986 | `tests/reference/` and unit tests | Exports the reference vectors once. Its checks become Rust tests |
| [FsHarness.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/build/FsHarness.cs), [Verify.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/build/Verify.cs) | ~460 | Headless golden renders | Replaced |

The screenshots in Nostalgia+'s `docs/` folder, copied to `tests/reference/screenshots/`, become the parity reference for the new renderer.

## Architecture

### Threads and data flow

```
capture thread (real-time)   analysis thread              render thread (main)
PipeWire / WASAPI callback   fixed hop, 120 per second    winit loop, vsync
        |                            |                            |
        +-- f32 frames + clock ----->+ FFT bank, loudness,        |
            (rtrb ring)              | true peak, features        |
                                     |                            |
                                     +-- history rows, scope ---->+ upload new data only
                                     |   samples, waveform        |
                                     |   (SPSC queue)             |
                                     +-- meters, curves --------->+ ballistics at real dt
                                         (triple buffer)          | wgpu passes, text, egui
media thread (zbus / WinRT) -- track, position, artwork -------->+ present
```

**Real-time rules:**

- **Capture thread:** the callback only copies samples into a pre-allocated
  single-producer ring (`rtrb`). It never allocates, locks, logs or makes system calls. It
  runs at whatever priority the audio API gives it: PipeWire's data loop through RTKit on
  Ubuntu, and MMCSS "Pro Audio" on Windows.
- **Analysis thread:** allocates only when a setting changes (FFT plans, buffer sizes). The
  hop is a fixed number of samples, so results never depend on the display.
- **Render thread:** never waits. It takes whatever the analysis last published. D-Bus,
  WinRT, HTTP, file I/O and image decoding all stay off it.
- **Timing:** everything that animates uses real elapsed time, never "per frame". That
  covers attack and release, peak decay, fades, phosphor decay and hue drift. Today several
  constants assume 60 fps, such as the onset and tempo windows in
  [MusicFeatures.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/src/Dsp/MusicFeatures.cs). They become seconds.
- **Idle:** when the window is hidden or covered, rendering stops. Analysis keeps running
  so that LUFS-I, LRA and overs stay correct. When nothing is playing, the frame rate
  drops.

### Crates

```
sonorant/
  Cargo.toml              workspace; rust-toolchain.toml pins stable
  crates/
    sonorant-dsp/         FFT bank, loudness + true peak, dynamic range, music features,
                          curve shaping, frequency map. No platform dependencies
    sonorant-core/        settings + presets, palettes, menu model, analysis engine
                          (threads, rings, clocks), AudioSource + MediaSession traits
    sonorant-platform/    linux/: PipeWire capture, MPRIS, portals (accent, inhibit)
                          windows/: WASAPI + process loopback, SMTC, accent, power
    sonorant-render/      wgpu: history store, spectrum, scope, bloom, waterfall,
                          backdrop, deck/band/quick bar layout, text
    sonorant/             the binary: winit loop, egui menus/help/dialogs, input,
                          windowed/fullscreen/immersive
  tests/reference/        vectors exported from the Nostalgia+ harness
  tests/golden/           headless render goldens
  packaging/
    flatpak/  debian/  windows/
```

| Need | Crate |
|---|---|
| Window, input, IME | winit |
| GPU | wgpu, with shaders in WGSL |
| Menus, help, dialogs | egui, egui-wgpu, egui-winit, AccessKit |
| Deck and axis text | cosmic-text and glyphon, with IBM Plex bundled |
| FFT | rustfft (AVX, SSE, NEON) |
| Lock-free queues | rtrb, triple_buffer |
| Linux capture | pipewire (pipewire-rs) |
| D-Bus for MPRIS and portals | zbus |
| WASAPI, SMTC, accent colour, power | windows (windows-rs) |
| Artwork | image, and ureq for `https://` art |
| Settings | serde and toml |

Versions are pinned in `Cargo.lock`. `cargo-deny` checks licences and security advisories
in CI. wgpu, winit and egui are always upgraded together.

### Rendering

Each frame runs these passes in order:

1. **Upload.** Only what's new: history rows since the last frame, scope samples, waveform
   columns and uniforms. Each row costs O(width), like today's ring.
2. **Backdrop**, when it's switched on.
3. **Spectrogram.** One quad per pane samples the history store and a 256-entry palette
   texture. The scroll offset is fractional and comes from the audio clock, so motion is
   smooth at any refresh rate.
4. **Waterfall**, which replaces pass 3 in the 3D view.
5. **Curves.** Spectrum, peak, average and minimum traces are drawn as anti-aliased strips,
   with signed-distance edges in the fragment shader. Bars and LED segments are instanced
   quads.
6. **Scope.** The goniometer is drawn into a floating-point persistence texture. Each frame
   decays it by elapsed time, then adds the new samples additively. Samples are upsampled
   4× so the traces are continuous, as on an analogue scope.
7. **Bloom.** A downsample/upsample chain over the glowing layers. It replaces today's
   `ColorMatrix`-plus-blur glow.
8. **Composite.** Tonemaps the floating-point buffer to the sRGB swapchain. Output is
   standard dynamic range.
9. **Text and UI.** Deck text and axis labels, then egui on top.

GPU timestamp queries time every pass. The status line shows fps, analysis time and GPU
time per pass, where today it shows fps, DSP time and paint time.

### History store

- **Rows hold values, not colours.** Each row stores dB values as Float16 on a fixed grid of
  2,048 log-spaced frequencies from 10 Hz to 24 kHz. The shader applies the palette, the
  frequency axis (note, log or linear, FMin to FMax) and the zoom. Changing any of them
  redraws the whole history at once instead of only the new rows.
- **Each row keeps its own range.** The floor and ceiling that auto-range had when a row was
  made are kept in a small side texture, so history looks exactly as it does today. A
  "global range" setting re-maps the whole history as the range adapts instead.
- **Capacity:** rows live in 4,096-row layers of a texture array. At the default 60 rows per
  second, 5 minutes is about 18,000 rows. That's about 74 MB per channel, or 150 MB for
  stereo. The length is a setting.
- **Zooming out stays cheap.** A compute pass builds a max-pooled mip chain along the time
  axis as rows arrive. A zoomed-out view reads a coarser level instead of aliasing, and
  costs the same as the live view.
- **Row metadata:** a CPU ring beside the store keeps each row's timestamp, track ID and
  position in the track. It drives the time axis, double-click-to-seek (only inside the
  current track) and the hover time offset. Hover levels come from a one-texel async
  readback. That arrives a frame late, which a readout never shows.

### UI layer

- **egui** draws the right-click menu, the help window and the name dialog, themed with
  Sonorant's colours and font, in the same frame as the visuals. They behave the same in
  windowed, fullscreen and immersive mode and need no second OS window, which Wayland
  couldn't place anyway.
- **The menu becomes a model.** The 1,468 lines of [MenuFactory.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/src/Ui/MenuFactory.cs)
  become a tree of items, each with a label, check state, radio group, shortcut, action and
  help text. egui renders the tree. The keyboard handler and the help search read the same
  tree, so a shortcut or help entry can't drift away from its menu item. The model is
  unit-tested, as the harness's "menu actions" section is today.
- **AccessKit** exposes the menus, help and dialogs to screen readers (Orca and Narrator).

### Platform layer

| | Ubuntu | Windows |
|---|---|---|
| Window and input | winit on Wayland (X11 fallback), fractional scaling | winit on Win32, per-monitor DPI |
| GPU backend | Vulkan (OpenGL fallback) | Direct3D 12 (Vulkan and OpenGL fallback) |
| Whole-system audio | PipeWire, monitor of the default sink | WASAPI loopback on the default render device |
| Just the player's audio | PipeWire, capturing the player's output stream node | WASAPI process loopback (build 19041 and later) |
| Now playing | MPRIS over zbus | SMTC through windows-rs |
| Accent colour, dark/light | XDG Settings portal (`color-scheme`, `accent-color`) | `UISettings` |
| Keep the screen on | Inhibit portal, idle flag | `SetThreadExecutionState` |
| Settings folder | `$XDG_CONFIG_HOME/sonorant/` | `%APPDATA%\Sonorant\` |

## Phases

Phase 0 has no dependencies. Phases 1 and 2 can run in parallel after it.

### Phase 0: repository, CI and a working skeleton

- Create the `sonorant` repo with the five-crate workspace. `rust-toolchain.toml` pins
  stable Rust, edition 2024. This plan moves there as its first commit.
- Run CI on `ubuntu-24.04`, `ubuntu-24.04-arm` and `windows-latest`: `cargo fmt`, `clippy`
  with warnings as errors, tests and `cargo-deny`.
- **Skeleton:** a winit window with wgpu and egui, showing a synthetic scrolling texture and
  a test menu. Measure frame pacing at 60 and 144 Hz on GNOME Wayland at 125% and 150%
  scaling, and on Windows 10 and 11.
- **Reference vectors:** in the Nostalgia+ repo, add `--export <dir>` to
  [TestHarness.cs](https://github.com/nifraz/NostalgiaPlus/blob/main/build/TestHarness.cs). It writes each DSP section's inputs as raw
  little-endian f64 and its outputs as JSON. Commit the output to
  `sonorant/tests/reference/`, and copy the screenshots from `docs/` alongside.
- **Done when:** the skeleton runs locked to vsync with no dropped frames on both OSes, and
  CI is green on all three runners.

### Phase 1: DSP, verified

- Write `sonorant-dsp`:
  - the FFT bank with all four profiles: Fast (4096), Balanced (16384, 4096, 1024), High
    (32768, 8192, 2048, 512) and Low latency (4096, 1024, 256), with crossover blending
  - both stereo channels packed into one complex FFT
  - the six window types, spectral tilt and rolling-percentile auto-range
  - BS.1770 loudness (M, S, I, LRA), 4× true peak, crest and overs
  - BPM and brightness, curve shaping, the frequency map and note naming
- Keep f64 throughout, as today, so the reference numbers match. Revisit f32 for the FFT
  only if profiling calls for it.
- **Analysis engine** in `sonorant-core`: capture ring, then a fixed hop, then publish. The
  hop rate is a parameter. Tests pin it to Nostalgia+'s 60 per second to compare against
  the references, and the app runs at 120. If profiling needs it, the large FFTs can update
  less often than the small ones.
- Port the harness's other sections as Rust tests: settings persistence, user presets,
  themes, menu actions, layout budgets and image timeline.
- Add 44.1 kHz cases. At that rate the loudness filters are computed rather than taken from
  the published 48 kHz values, and they land about 0.2 dB off.
- Benchmark with `criterion`.
- **Done when:** every reference vector matches (spectra within 0.01 dB above −120 dBFS,
  loudness within 0.01 LU, and overs, notes and BPM exactly), and a Balanced stereo hop
  takes under 0.5 ms.

### Phase 2: capture

- **`AudioSource` trait:** start and stop, sample rate, channel layout, a clock, and a
  status: running, no audio server, no device, suspended, or exclusive mode.
- **Ubuntu:** a pipewire-rs stream capturing the default sink's monitor. It follows changes
  to the default sink, such as Bluetooth headphones connecting, and asks for a 256-sample
  quantum to keep latency low.
- **Ubuntu, one app:** capture a chosen app's output stream node directly. For now a manual
  "Capture from" list drives it. Phase 4 matches it to the player automatically.
- **Windows:** event-driven WASAPI loopback on an MMCSS thread, process loopback through
  `ActivateAudioInterfaceAsync`, default-device changes and the exclusive-mode message.
- **WAV source:** a file source that feeds the same pipeline deterministically. Tests,
  golden renders and demos use it.
- **Done when:** live playback drives analysis on Ubuntu 24.04 and 26.04 and on Windows 10
  (19045) and 11, both whole-system and single-app, with audio reaching analysis in under
  15 ms.

### Phase 3: renderer, parity

- The history store, the spectrogram with fractional scrolling, and the palette textures:
  Magma, Inferno, Viridis, Turbo, Ice, Grey and Nostalgia Red.
- The curves, the goniometer, correlation and balance bars, waveform lanes, colour bar and
  status line. See the [feature checklist](#feature-checklist) for every style and option.
- The floating-point target and bloom (today's glow), plus the backdrop, hue drift and beat
  flare at today's level.
- **Layout:** the centre deck, bottom band and quick bar, rebuilt on Nostalgia+'s layout
  rules and their tests. For example, the artwork goes first when space runs out, and
  readouts fill columns before adding new ones.
- **Text:** cosmic-text and glyphon, with IBM Plex Sans and Plex Mono bundled and system
  font fallback.
- **Golden renders:** offscreen wgpu on software renderers in CI (lavapipe on Ubuntu, WARP
  on Windows), fed from the WAV source and compared with a tolerance.
- **Done when:** the windowed, fullscreen and immersive views match Nostalgia+ in
  content and layout, and a frame at 2560×1440 takes under 3 ms of GPU time on an
  integrated GPU. (Judged against Nostalgia+'s current code and its exported layouts:
  the screenshots predate the centre deck. 1440p needs offscreen timing, in Phase 7.)

### Phase 4: now playing

- **`MediaSession` trait:** the list of players and the active one, metadata, artwork, play
  state, position, capabilities and transport commands.
- **MPRIS over zbus** supplies every field the deck uses:
  - title, artists, album and composer from the `xesam:*` metadata fields
  - year from the first four digits of `xesam:contentCreated`
  - length from `mpris:length`, which is in microseconds
  - artwork from `mpris:artUrl`, reading `file://` URLs directly and fetching and caching
    `https://` ones (Spotify and browsers use these)
- **Position:** MPRIS doesn't signal position changes. Cache the position and extrapolate
  it, resyncing on the `Seeked` signal and about once a second. Seeking uses
  `SetPosition(trackId, µs)`. When `CanSeek`, `CanGoNext` or `CanControl` is false, the
  matching control stays unwired and the deck hides it.
- **SMTC** through `GlobalSystemMediaTransportControlsSessionManager`. Its timeline
  properties are extrapolated the same way, and artwork comes from the session's thumbnail.
- A new track ID triggers today's track-change resets: LUFS-I, LRA, BPM and overs.
- By default Sonorant follows whichever player is playing. A "Follow player" submenu lets
  the user pin one.
- **Capture follows the player:**
  - On Ubuntu, get the player's process ID from D-Bus (`GetConnectionUnixProcessID`) and
    match it to the PipeWire node's `application.process.id`. Fall back to matching the
    app name, and then to the whole mix.
  - On Windows, map the SMTC app ID to its process and use process loopback. Fall back to
    system loopback.
  - The status line shows what is being captured. A Capture menu offers "Following player"
    or "Whole system".
- **Done when:** tested against Rhythmbox, Strawberry, Spotify, Firefox and VLC on Ubuntu,
  and against MusicBee, Spotify and a browser on Windows.

### Phase 5: app shell

- **Views:**
  - Windowed mode is the old docked panel.
  - Fullscreen (F11 or Esc) opens on the monitor the window is on.
  - Immersive mode (`I`) keeps the furniture fade, cursor hiding and cinematic mode.
- **Interaction:** every existing key, the hover readouts, synced hover and the hover pin,
  freeze, the amber reference curve and double-click to seek.
- The egui menu is built from the model, and the help search and name dialog carry over.
- **Settings and presets:** Studio, Nostalgia, QC, Immersive and user presets. On first run
  on Windows, import `NostalgiaPlus\NostalgiaPlus.settings` from both MusicBee locations:
  `%APPDATA%\MusicBee\` for the installer build and the package folder for the Microsoft
  Store build.
- **Colours:** the OS accent colour and dark/light preference replace MusicBee's skin
  colours. The theme's colour slots carry over.
- **HiDPI:** lay out in logical pixels, but keep the history, scope and waveform in
  physical pixels so they keep their detail at 150%.
- Keep the screen awake while fullscreen and playing.
- **Done when:** the parity checklist passes on GNOME Wayland at 100%, 125% and 200%
  scaling, and on Windows 10 and 11. Testers get a parity preview build, not a release.

### Phase 6: new visuals

- **Phosphor scope:** the goniometer's persistence decays with real time and is drawn
  additively with bloom. The curves can optionally use it too. Settings: persistence and
  intensity.
- **Zoomable long history:** the mouse wheel zooms time around the cursor and dragging pans
  back through it. A Live button and the End key return to the live view, and freeze pairs
  naturally with this. History length is a setting, from 1 to 15 minutes.
- **3D waterfall:** a new view alongside panes and mirror, with its own key. A grid mesh is
  displaced in the vertex shader straight from the history store, so the CPU never
  rebuilds a mesh. Normals come from neighbouring values, colour from the palette, plus
  distance fog. Drag to orbit and use the wheel to zoom, with a few preset cameras.
- **Beat-reactive backdrop:** a full-screen shader driven by BPM phase, onsets, brightness
  and hue drift. It extends today's backdrop settings: backdrop on/off and strength, beat
  reactive, and colour follows.
- **Quality setting** (Low, Medium, High): scales bloom depth, waterfall mesh density and
  backdrop detail for weak GPUs.
- **Done when:** each visual has golden images, and with everything on a frame stays under
  6 ms of GPU time at 1440p on an integrated GPU.

### Phase 7: tuning and polish

- **Frame pacing:** check 60, 144 and 240 Hz monitors and variable refresh rate. Use
  mailbox presentation where the platform offers it.
- **Latency:** measure end to end with timestamps through the pipeline.
- **Power:** a low idle frame rate when paused, a full stop when the window is covered, and
  a check on battery.
- **Visual delay:** an offset so the visuals line up with what you hear: set by hand, and
  filled in automatically on Ubuntu where PipeWire reports the sink's latency.
- **Accessibility:** a pass with Orca and Narrator.
- **Weak GPUs:** check the OpenGL backend and software rendering.
- **Done when:** the [targets](#targets) are met on the two reference machines.

### Phase 8: packaging and release

- **Flatpak on Flathub** as `io.github.nifraz.Sonorant`, with these permissions:
  - `--socket=wayland`, `--socket=fallback-x11` and `--device=dri`
  - `--filesystem=xdg-run/pipewire-0`, for native PipeWire
  - `--talk-name=org.mpris.MediaPlayer2.*`
  - `--share=network`, for `https://` artwork
- **`.deb` packages** for x64 and arm64 on GitHub Releases, depending on
  `libpipewire-0.3-0`.
- **Linux metadata:** an AppStream file, a `.desktop` entry and an icon.
- **Windows:** a signed zip download and a winget package. The Microsoft Store comes later.
- **Release workflow:** builds both OSes and stamps the version from the tag into
  `Cargo.toml`.
- **The Nostalgia+ repo:** tag the last plugin build `plugin-final` and point the README to Sonorant.
  The last `mb_NostalgiaPlus.dll` release stays downloadable.
- **Skip Snap.** Its `audio-record` permission isn't connected automatically, so capture
  wouldn't work out of the box.

## Feature checklist

### DSP (Phase 1)

- [ ] Multi-resolution FFT: Fast, Balanced, High and Low latency profiles
- [ ] Both stereo channels in one complex FFT
- [ ] Windows: Hann, Hamming, Blackman-Harris, Nuttall, Gaussian and rectangular
- [ ] Peak or energy band aggregation, spectral tilt
- [ ] Rolling-percentile auto-range
- [ ] Channel pairs: left/right, mid/side, left only, right only
- [ ] Note, log and linear frequency axes; note ± cents readout
- [ ] Attack and release, peak decay, averaging
- [ ] LUFS-M, LUFS-S, LUFS-I, LRA
- [ ] True peak, crest, overs
- [ ] BPM, brightness

### Drawing (Phase 3)

- [x] Scrolling spectrogram, scroll speeds and cinematic mode
- [x] Palettes: Magma, Inferno, Viridis, Turbo, Ice, Grey, Nostalgia Red
- [x] Curve styles: line, bars and LED. Interpolation: flat peaks, linear-smooth and cubic
      spline. Filtering from none to strong
- [x] Peak, average and minimum traces; solid fill
- [x] Graph backgrounds: plain, lines, grid, chessboard
- [x] dB scale, time marks, semitone lines, axis labels; harmonics with the hover
      readout in Phase 5
- [x] Goniometer, correlation and balance bars
- [x] Waveform lanes
- [x] Glow (now bloom), hue drift, beat flare; the backdrop with artwork in Phase 4
- [x] Colour bar and status line
- [x] Centre deck and bottom band, with today's layout rules; the quick bar with its
      buttons in Phase 5
- [x] Theme colour slots

### Now playing (Phase 4)

- [x] Artwork, title, composer, artists, album, year
- [x] Transport, seek bar, clock
- [x] Double-click a spectrogram column to seek there
- [x] Per-track resets

### App shell (Phase 5)

- [x] Windowed view (the old docked panel)
- [x] Fullscreen mirrored stereo view
- [x] Immersive mode, including furniture fade, cursor hiding and cinematic mode
- [x] Keys: `Esc`/`F11`, `I`, `Space`, `A`, `W`, `O`, `G`, `P`, and `B`, `C` and `F1`
      beside them
- [x] Hover readout: frequency, note ± cents, level for each channel, time offset. Synced
      hover and hover pin
- [x] Freeze, amber reference curve
- [x] Right-click menu, including the centre deck switches, graph size, deck height and
      curve width
- [x] Help window with search, name dialog
- [x] Presets: Studio, Nostalgia, QC, Immersive, and user presets

### Replaced

- [x] MusicBee skin colours become the OS accent colour and dark/light preference
- [x] The MusicBee "Toggle fullscreen" command becomes F11 inside the app. A global
      shortcut can follow later (the GlobalShortcuts portal on Linux, `RegisterHotKey` on
      Windows)
- [x] `TargetFps` becomes a frame-rate cap (display, 60 or 30), and `ScrollDivider`
      becomes a scroll speed in rows per second

### New in Sonorant

- [x] Display-rate rendering with scrolling driven by audio time
- [x] Capture only the player's audio
- [x] Palette, axis and zoom changes redraw the whole history
- [ ] Phosphor scope with bloom
- [ ] Zoomable long history
- [ ] 3D waterfall view
- [ ] Beat-reactive shader backdrop
- [x] Screen stays awake while fullscreen and playing
- [ ] Menus, help and dialogs readable by screen readers (AccessKit is attached; nothing
      has been read with Orca or Narrator yet — Phase 7)

## Risks

| Risk | Mitigation |
|---|---|
| wgpu, winit and egui ship breaking releases every few months, and their versions have to line up | Pin them, and upgrade all three together in one change on a schedule |
| Older or weaker GPUs: no Vulkan, little video memory | wgpu's OpenGL backend, the quality setting and the history-length setting. CI already runs on software renderers |
| v1 is large: parity plus four new visuals | The new visuals come after the parity gate, and testers get a preview build after Phase 5. A visual that isn't ready moves to 1.1 without holding up the release |
| MusicBee may not publish to SMTC. If it doesn't, MusicBee users on Windows lose the now-playing half of the deck | Check at the start of Phase 4. The fallback is a tiny bridge plugin that feeds Sonorant |
| Sandboxed players (Flatpak apps, browsers) often don't match by process ID, and Sonorant's own sandbox can hide process IDs | Fall back to matching by name, then to the whole mix. The status line says which one is in use |
| Wayland won't let an app choose where its window goes or keep it on top | Fullscreen opens on the window's monitor. An always-on-top compact mode only where the platform allows it (Windows, X11) |
| Track titles in scripts the bundled font doesn't cover | System font fallback through cosmic-text |
| Flathub reviewers ask about `xdg-run/pipewire-0` | Other PipeWire audio tools on Flathub use the same permission. Explain it in the submission |
| Unsigned Windows binaries trigger SmartScreen warnings | Code signing through SignPath Foundation |
| Loudness about 0.2 dB off at 44.1 kHz | 44.1 kHz cases in Phase 1 |
| MPRIS support varies by player (seek, artwork, composer) | Unwired controls stay hidden, as the deck already does |
| "sonorant" is also an audiology product (an LED ear light), and Rogers Imaging filed an "Sonorant" trademark in 2020 for hearing and lighting devices | Software is a different trademark class. Use "Sonorant – Music Visualizer" as the store and search title. Not legal advice |

## How the open questions were settled

The plan left eleven questions open, and setting up Phase 0 added a twelfth, the licence.
All twelve were answered on 2026-09-19. Each took the suggested answer except the font,
and the answers are in the [decisions](#decisions) table. What's worth keeping from the
reasoning:

- **Licence:** GPL-3.0-or-later keeps forks and store re-uploads open, and SignPath's free
  signing needs an OSI-approved licence. Nostalgia+ had no licence file.
- **Font:** IBM Plex Sans with Plex Mono, rather than the suggested Inter, for an
  engineering character that suits a metering tool. Plex Mono sets the readouts, so
  changing numbers don't jitter.
- **Targets:** the development PC is the low-end floor. Its integrated GPU has no
  Direct3D 12 or Vulkan driver, so it is measured on the GeForce 840M.
- **History:** with the time-axis mip chain, 5 minutes of stereo history takes about
  295 MB of GPU memory, right at the 300 MB target. Phase 3 measures it; the length
  setting is the lever if it's over.
- **Ubuntu 22.04** is out: it still sends audio through PulseAudio by default, so native
  PipeWire capture sees nothing there.
- **Windows 10** keeps single-app capture: process loopback works from build 19041.
  Windows 10 left mainstream support in October 2025 and consumer security updates end
  in October 2026.

## Appendix: choosing the name

**Chosen:** Sonorant. In phonetics, a sonorant is a sound made with unobstructed,
voiced airflow: the vowels, nasals and liquids, the parts that carry pitch and can be
sung. No software or audio product was found using the name.

**Clear but not chosen:** Tonebloom, Halation, Afterhue, Wavelume, Prismtone, Glowmoth,
Diptych, Mirrorscope, Twinscope, Sonoglow, Glowtrace, Mirrorlume, Twinhue.

**Rejected because the name is already in use:**

- Earlight: a trademark filing by Rogers Imaging Corporation.
- Earsight: an assistive audio app on Google Play, a GitHub project of the same name,
  and a field-recording microphone maker.
- Audio or visualiser products with the same name:
  - Chromascope: a VST3 spectrum plugin
  - Spectrolite: an iOS spectrogram app with a note-scale view
  - Chromatone and Timbra: both include spectrogram tools
  - Lumisonic: a visualiser
  - Phosphene and Phosphor: several visualisers
  - Iridia: a reverb plugin
  - ToneLens: an iOS chord app
  - Sonalux: a music-tech company
- Other clashes:
  - Afterglow: PDP's app on the Microsoft Store and a macOS screensaver app
  - Stereograph: an existing Linux project
  - Overtone: a Clojure music library
  - Nostalgix: a card game and a retro handheld
  - Luminote, Otoiro, Fermata, Nightbloom, Emberline, Songlight, Noctiluca, Spectrail,
    Wavesight

## References

- [wgpu](https://wgpu.rs), [winit](https://github.com/rust-windowing/winit),
  [egui](https://github.com/emilk/egui), [AccessKit](https://accesskit.dev)
- [cosmic-text](https://github.com/pop-os/cosmic-text),
  [glyphon](https://github.com/grovesNL/glyphon)
- [RustFFT](https://github.com/ejmahler/RustFFT), [rtrb](https://github.com/mgeier/rtrb)
- [pipewire-rs](https://gitlab.freedesktop.org/pipewire/pipewire-rs),
  [zbus](https://github.com/dbus2/zbus), [windows-rs](https://github.com/microsoft/windows-rs)
- [MPRIS specification](https://specifications.freedesktop.org/mpris-spec/latest/)
- [Application loopback sample](https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/ApplicationLoopback):
  WASAPI process loopback
- [ITU-R BS.1770](https://www.itu.int/rec/R-REC-BS.1770): loudness and true peak
- [Rogers Imaging Corporation trademarks](https://trademarks.justia.com/owners/rogers-imaging-corporation-5469822):
  the "Earlight" filing
- [EarSight](https://maxloh.com/earsight/): the assistive audio app of that name
- [Afterglow on the Microsoft Store](https://apps.microsoft.com/detail/9mvk44x6r37d)
