#!/usr/bin/env bash
# Puts a version into Cargo.toml and Cargo.lock: packaging/stamp-version.sh 1.2.3
#
# The tag is what a release is named by, so the tag is what the binaries should report.
# Everything downstream reads the version out of Cargo.toml (the `.deb`, the zip, the
# winget manifests, `sonorant --version`), so stamping it here is the only place a
# release's number is written.
#
# Cargo.lock names the workspace's own crates too, so it is stamped as well: without
# that, every `--locked` build after this would refuse to start.
set -euo pipefail

version=${1:-}
if [ -z "$version" ]; then
    echo "usage: packaging/stamp-version.sh <version>" >&2
    exit 2
fi
# What Cargo accepts and what a tag looks like: 1.2.3, and pre-release suffixes too.
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'; then
    echo "not a version: $version" >&2
    exit 2
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

# Only the `version` in `[workspace.package]`, not `rust-version` and not a
# dependency's version further down the file.
awk -v v="$version" '
    /^\[/ { in_block = ($0 == "[workspace.package]") }
    in_block && /^version = / && !done { print "version = \"" v "\""; done = 1; next }
    { print }
' Cargo.toml > Cargo.toml.stamped
mv Cargo.toml.stamped Cargo.toml

# --workspace touches the workspace's own packages and leaves every dependency where
# the lock file has it, so this cannot quietly bring in a new version of anything.
cargo update --workspace --quiet

grep -A1 '^\[workspace.package\]' Cargo.toml | tail -1
