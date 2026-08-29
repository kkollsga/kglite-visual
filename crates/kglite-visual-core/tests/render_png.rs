//! The PNG path's smoke test.
//!
//! **Not a golden.** A rasteriser's output depends on the fonts installed on
//! the machine that ran it, and pinning bytes here would pin this developer's
//! font list. The exact baseline lives on the SVG, which is what the PNG is
//! rendered *from* (`render_golden.rs`); what is left to check is that the
//! rasteriser ran, produced a real PNG of the size that was asked for, and drew
//! something.
//!
//! Decoded with the `png` crate rather than with resvg's own encoder read back:
//! an encoder checked only by its own decoder can agree with itself about a
//! file nothing else opens.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kglite_visual_core::render::{self, RenderFormat, RenderRequest, RenderSource, Theme};
use kglite_visual_core::{load_graph, GraphSource, QueryConfig};

fn fixture_graph() -> Arc<kglite::api::DirGraph> {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("meta.kgl");
    load_graph(GraphSource::Path(&path)).expect("the committed fixture loads")
}

fn png_request(width: u32, height: u32) -> RenderRequest {
    RenderRequest {
        source: RenderSource::Meta,
        format: RenderFormat::Png,
        width,
        height,
        seed: 1234,
        theme: Theme::Dark,
    }
}

#[test]
fn a_png_render_decodes_at_the_dimensions_that_were_asked_for() {
    let graph = fixture_graph();
    let rendered = render::render(
        &graph,
        "meta.kgl",
        QueryConfig::default(),
        &png_request(640, 480),
    )
    .expect("the fixture rasterises");

    assert_eq!(rendered.format, RenderFormat::Png);
    assert!(
        rendered.bytes.len() > 1_024,
        "a 640x480 graph is not 1 KB of PNG; got {} bytes",
        rendered.bytes.len()
    );
    assert_eq!(
        &rendered.bytes[..8],
        b"\x89PNG\r\n\x1a\n",
        "the PNG signature, before any decoder is trusted"
    );

    let decoder = png::Decoder::new(std::io::Cursor::new(&rendered.bytes));
    let mut reader = decoder.read_info().expect("a decoder that is not resvg");
    let info = reader.info();
    // `--width 640` is a promise, not a suggestion: the emitter writes a
    // matching viewBox so the raster transform is the identity.
    assert_eq!((info.width, info.height), (640, 480));

    let mut buffer = vec![0u8; reader.output_buffer_size().expect("a bounded frame")];
    let frame = reader.next_frame(&mut buffer).expect("one frame decodes");
    let pixels = &buffer[..frame.buffer_size()];

    // Something was drawn. The background is `#0d1117`; a pixmap that came back
    // uniformly *anything* would decode, be the right size, and be a picture of
    // nothing — which is exactly what a silently-missing renderer produces.
    let first = pixels.as_chunks::<4>().0.first().copied().unwrap_or([0; 4]);
    assert!(
        pixels.as_chunks::<4>().0.iter().any(|px| *px != first),
        "every pixel is identical; nothing was rendered onto the canvas"
    );
}

/// The PNG and the SVG are the same picture, produced by one drawing function.
///
/// Observable here as: both formats report the same counts and the same banner
/// for the same request. A PNG path that had grown its own scene builder would
/// be free to disagree, and nothing about the bytes would say so.
#[test]
fn the_two_formats_answer_the_same_question() {
    let graph = fixture_graph();
    let png = render::render(
        &graph,
        "meta.kgl",
        QueryConfig::default(),
        &png_request(640, 480),
    )
    .expect("png");
    let svg = render::render(
        &graph,
        "meta.kgl",
        QueryConfig::default(),
        &RenderRequest {
            format: RenderFormat::Svg,
            ..png_request(640, 480)
        },
    )
    .expect("svg");

    assert_eq!((png.nodes, png.links), (svg.nodes, svg.links));
    assert_eq!(png.banners, svg.banners);
    assert_eq!((png.width, png.height), (svg.width, svg.height));
    assert_eq!(png.format.content_type(), "image/png");
    assert_eq!(svg.format.content_type(), "image/svg+xml");
}
