# Packaging

What a release is made of, and how to make one.

```
packaging/
  icons/        the icon, drawn by sonorant-render and committed here
  linux/        the .desktop entry and the AppStream description
  flatpak/      the Flatpak manifest and its generated crate list
  deb/          the .deb build and its copyright file
  windows/      the zip build
  winget/       the three manifests a winget submission is made of
  screenshots/  the pictures the AppStream description points at
  stamp-version.sh
```

## Making a release

Tagging is the whole procedure. `.github/workflows/release.yml` stamps the version from
the tag into `Cargo.toml` and `Cargo.lock`, builds a `.deb` on x86-64 and on arm64 and a
zip on Windows, checks that the program inside the package reports the version the tag
asked for, and publishes the lot with a `SHA256SUMS`.

```sh
git tag -a v0.1.0 -m 'Sonorant 0.1.0'
git push origin v0.1.0
```

To try it without publishing anything, run the workflow by hand from the Actions tab and
give it a version. It builds and checks everything and stops before the release is
created, leaving the packages as artifacts.

The version in `Cargo.toml` is only stamped inside the workflow's checkout, so the tag
stays the single place a release's number is written and `main` keeps whatever version
it had. To do the same locally: `packaging/stamp-version.sh 0.1.0`.

## Building a package by hand

```sh
packaging/deb/build.sh                      # target/deb/sonorant_<version>_<arch>.deb
packaging/windows/build.ps1                 # target\zip\sonorant-<version>-windows-x64.zip
flatpak-builder --user --install --force-clean build \
    packaging/flatpak/io.github.nifraz.Sonorant.yml
```

The `.deb` is built natively on each architecture rather than cross-compiled, which is
why the release workflow uses an arm64 runner for the arm64 package.

## The icon

`packaging/icons` is generated, not drawn by hand:

```sh
cargo run -p sonorant-render --example icon
```

The shapes live in `sonorant-render`'s `icon` module and are coloured out of Magma, the
default palette, so the icon is made of the same colours as the picture it stands for.
`cargo test -p sonorant-render --test icon` redraws the committed files and fails if
they have drifted.

## The Flatpak crate list

A Flatpak build has no network, so every crate is fetched by flatpak-builder first:

```sh
packaging/flatpak/cargo-sources.py           # rewrite it after a dependency bump
packaging/flatpak/cargo-sources.py --check   # what CI runs
```

It reads Cargo.lock and nothing else, checksums included, so it needs no network either.

## The screenshots

They are the app's own `--screenshot`, run against the reference signal the DSP tests
use, so they can be taken again on any machine:

```sh
# tests/reference/signals/music48.f64 is interleaved stereo f64 at 48 kHz; wrap it in a
# WAV header (tag 3, 64-bit float) and point the app at it.
sonorant --wav music48.wav --settings /tmp/shot --screenshot main.png --screenshot-seconds 9
```

**Turn off `deck_show_artwork`, `deck_show_track_info` and `imm_backdrop` first.**
Sonorant follows whatever is playing, so with a player running on the machine a
screenshot will otherwise carry that machine's album art and track title into a file
meant for a store listing. The three committed pictures were taken with those off.

## Still open

- **No code-signing certificate yet.** [SignPath Foundation](https://signpath.org/) is
  the route the plan decided on and has not been applied for, so the Windows zip is
  unsigned and SmartScreen warns the first people to run it. `build.ps1` signs if it is
  given a certificate and a machine with `signtool.exe` on PATH.
- **The Flatpak build has not been run**: there is no flatpak-builder on the machine
  this was written on. The manifest and the crate list are checked as far as they can
  be (the list against Cargo.lock, in CI) but nobody has watched it build.
- **Flathub's submission** is a pull request against `flathub/flathub` with the manifest
  above, its source swapped from `dir` to the release tag.
- `appstreamcli --pedantic` notes the capital S in `io.github.nifraz.Sonorant`. The ID
  is kept as the plan decided it: the last part of an AppStream ID is conventionally the
  app's own name, and GNOME Software and Flathub both match it case-sensitively against
  the `.desktop` file, which agrees.
