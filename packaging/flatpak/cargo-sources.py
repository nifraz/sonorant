#!/usr/bin/env python3
"""Turns Cargo.lock into the source list a Flatpak build downloads.

A Flatpak build has no network, so every crate has to be listed as a source
flatpak-builder fetches beforehand. Cargo.lock already carries the sha256 of each
registry crate, so this needs nothing but the lock file: no crates.io queries, no
cargo, and the same output on every machine.

    packaging/flatpak/cargo-sources.py            rewrite cargo-sources.json
    packaging/flatpak/cargo-sources.py --check    fail if it is out of date

The upstream tool (flatpak-builder-tools/cargo) does the same job and handles git
dependencies as well. This one deliberately refuses them instead: the workspace has
none, and a build whose sources come from a lock file alone is one nobody has to have
network access to reproduce.
"""

import argparse
import json
import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
LOCK = ROOT / "Cargo.lock"
OUT = pathlib.Path(__file__).resolve().parent / "cargo-sources.json"

# Where CARGO_HOME points during the build, relative to the module's build directory.
CARGO_HOME = "cargo"
VENDOR = f"{CARGO_HOME}/vendor"

# What cargo is told, so it takes the vendored crates and never looks at the network.
CONFIG = f"""\
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "{VENDOR}"
"""


def sources(lock: dict) -> list:
    out = []
    for package in sorted(lock["package"], key=lambda p: (p["name"], p["version"])):
        source = package.get("source")
        if source is None:
            continue  # One of ours, built from the checkout itself.
        if not source.startswith("registry+"):
            raise SystemExit(
                f"{package['name']} {package['version']} comes from {source}; "
                "only crates.io is handled here"
            )
        name, version = package["name"], package["version"]
        checksum = package["checksum"]
        dest = f"{VENDOR}/{name}-{version}"
        out.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": checksum,
                "dest": dest,
            }
        )
        # Cargo checks a vendored crate against this file before it will use it. The
        # file list is empty on purpose: that tells cargo the crate is unmodified and
        # saves hashing every file in it.
        out.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": dest,
                "dest-filename": ".cargo-checksum.json",
            }
        )
    out.append(
        {
            "type": "inline",
            "contents": CONFIG,
            "dest": CARGO_HOME,
            "dest-filename": "config.toml",
        }
    )
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="don't write anything; fail if the file doesn't match Cargo.lock",
    )
    args = parser.parse_args()

    with LOCK.open("rb") as f:
        wanted = json.dumps(sources(tomllib.load(f)), indent=2) + "\n"

    if args.check:
        have = OUT.read_text() if OUT.exists() else ""
        if have != wanted:
            print(
                f"{OUT.relative_to(ROOT)} is out of date with Cargo.lock: "
                "run packaging/flatpak/cargo-sources.py",
                file=sys.stderr,
            )
            return 1
        print(f"{OUT.relative_to(ROOT)} is in step with Cargo.lock")
        return 0

    OUT.write_text(wanted)
    crates = sum(1 for s in json.loads(wanted) if s["type"] == "archive")
    print(f"{OUT.relative_to(ROOT)}: {crates} crates")
    return 0


if __name__ == "__main__":
    sys.exit(main())
