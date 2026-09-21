//! Writes the icon files the packaging installs: `cargo run -p sonorant-render --example icon`.
//!
//! They live in `packaging/icons` and are committed, because a `.deb` and a Flatpak
//! build have no reason to run a Rust example first. `icon_files_match_the_drawing` in
//! this crate's tests redraws them, so a stale file is a failing test rather than a
//! wrong icon on someone's desktop.

use std::path::PathBuf;

use sonorant_render::icon;
use sonorant_render::readback::write_png;

fn main() -> Result<(), String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("packaging/icons");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    for size in icon::SIZES {
        let path = dir.join(format!("sonorant-{size}.png"));
        write_png(&path, size, size, &icon::rgba(size))?;
        eprintln!("{}", path.display());
    }

    let svg = dir.join("sonorant.svg");
    std::fs::write(&svg, icon::svg()).map_err(|e| format!("{}: {e}", svg.display()))?;
    eprintln!("{}", svg.display());

    let ico = dir.join("sonorant.ico");
    std::fs::write(&ico, icon::ico()?).map_err(|e| format!("{}: {e}", ico.display()))?;
    eprintln!("{}", ico.display());
    Ok(())
}
