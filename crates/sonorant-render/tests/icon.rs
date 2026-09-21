//! The committed icon files against the drawing they came from.
//!
//! `packaging/icons` is what the `.deb`, the Flatpak and the Windows zip install, and
//! it is committed so that none of them has to run a Rust example first. This checks
//! the files still say what `icon.rs` draws, so retuning the icon and forgetting to run
//! `cargo run -p sonorant-render --example icon` is a failing test here rather than a
//! stale picture on someone's desktop.
//!
//! Pixels are compared rather than file bytes: a new version of the PNG encoder may
//! deflate the same image differently, and that isn't a drifted icon.

use std::path::PathBuf;

use sonorant_render::icon;

fn icons_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("packaging/icons")
}

fn read_png(path: &PathBuf) -> (u32, u32, Vec<u8>) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().expect("PNG header");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("PNG pixels");
    assert_eq!(info.color_type, png::ColorType::Rgba, "{}", path.display());
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[test]
fn every_committed_png_matches_the_drawing() {
    for size in icon::SIZES {
        let path = icons_dir().join(format!("sonorant-{size}.png"));
        let (width, height, pixels) = read_png(&path);
        assert_eq!((width, height), (size, size), "{}", path.display());
        assert_eq!(
            pixels,
            icon::rgba(size),
            "{} is stale: run `cargo run -p sonorant-render --example icon`",
            path.display()
        );
    }
}

#[test]
fn the_committed_svg_and_ico_match_the_drawing() {
    let svg = icons_dir().join("sonorant.svg");
    assert_eq!(
        std::fs::read_to_string(&svg).expect("sonorant.svg"),
        icon::svg(),
        "{} is stale: run `cargo run -p sonorant-render --example icon`",
        svg.display()
    );

    // The `.ico` holds PNGs, so it is compared the same way: the directory as written,
    // and each entry's pixels against a fresh draw.
    let path = icons_dir().join("sonorant.ico");
    let bytes = std::fs::read(&path).expect("sonorant.ico");
    assert_eq!(&bytes[0..6], &icon::ico().expect("ico")[0..6]);
    for (i, size) in icon::SIZES.iter().enumerate() {
        let entry = 6 + i * 16;
        let len = u32::from_le_bytes(bytes[entry + 8..entry + 12].try_into().unwrap()) as usize;
        let at = u32::from_le_bytes(bytes[entry + 12..entry + 16].try_into().unwrap()) as usize;
        let decoder = png::Decoder::new(std::io::Cursor::new(&bytes[at..at + len]));
        let mut reader = decoder.read_info().expect("PNG header");
        let mut buf = vec![0; reader.output_buffer_size().expect("size")];
        let info = reader.next_frame(&mut buf).expect("PNG pixels");
        buf.truncate(info.buffer_size());
        assert_eq!((info.width, info.height), (*size, *size));
        assert_eq!(
            buf,
            icon::rgba(*size),
            "{} is stale at {size}: run `cargo run -p sonorant-render --example icon`",
            path.display()
        );
    }
}
