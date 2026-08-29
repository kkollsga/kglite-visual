//! PNG, rasterised from the SVG this crate just emitted (plan D13).
//!
//! **PNG is derived, never authored.** There is one drawing function
//! ([`super::svg::emit`]) and this rasterises its output, so a PNG and an SVG
//! of the same request cannot disagree about anything. No JPEG, ever — it is
//! the wrong codec for line art, and a graph is nothing but line art.
//!
//! **`resvg` 0.48, pure Rust, MIT/Apache-2.0, MSRV 1.85** (below this
//! workspace's 1.88, which is kglite's). `tiny_skia` comes through resvg's own
//! re-export rather than a second manifest entry: two independent version
//! requirements on the same rasteriser is a place a `cargo update` can produce
//! two copies of it, one of which the renderer does not use.
//!
//! **Fonts: the system's, resolved explicitly, and a loud failure when there
//! are none.** resvg needs real font data to lay out `<text>`, and it has no
//! opinion about missing fonts: it drops the glyph run and renders the rest, so
//! a fontless machine produces a perfectly valid PNG of an unlabelled graph —
//! the silent-wrong-output shape this project bans. Hence [`fonts`] below,
//! which resolves a concrete family for the generic `sans-serif` /`monospace`
//! the emitter asks for and refuses outright when the database came back empty.
//!
//! Measured cost of the decision (release binary, this machine, 2026-08-29):
//! **11 739 152 -> 14 105 184 bytes, +2.26 MiB.** That is resvg + usvg +
//! tiny-skia + the text stack (fontdb, harfrust, skrifa); `svgz` and
//! `raster-images` are turned off because this emitter produces neither. The
//! alternative considered and rejected was embedding a font: a Latin-only
//! DejaVu Sans is ~757 KB of *additional* binary on every install to cover a
//! case this code already detects and reports, and it would still not be the
//! font the SVG names, so the PNG and a browser's view of the same SVG would
//! diverge. If a fontless deployment ever turns out to be common, the embedded
//! face is the fix and the error message below is where it would be wired in.

use std::sync::{Arc, OnceLock};

use resvg::tiny_skia;
use resvg::usvg;

use crate::error::CoreError;

/// Concrete families to back the emitter's generic `sans-serif`, most
/// preferred first.
///
/// The stack the SVG names (`ui-sans-serif, system-ui, …, sans-serif`) is a CSS
/// stack, and its first two entries are CSS keywords no font database has an
/// entry for. usvg resolves the tail generic against
/// `fontdb`'s configured family, which defaults to "Times New Roman" — a serif,
/// and on most Linux boxes absent — so leaving it alone would silently change
/// the typeface or lose the text.
const SANS_CANDIDATES: [&str; 8] = [
    "Helvetica Neue",
    "Helvetica",
    "Arial",
    "Segoe UI",
    "Roboto",
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
];

/// The same, for the status block's monospace stack.
const MONO_CANDIDATES: [&str; 6] = [
    "SF Mono",
    "Menlo",
    "Consolas",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Courier New",
];

/// The system font database, loaded once.
///
/// `load_system_fonts` walks the platform's font directories and parses every
/// face it finds — tens of milliseconds to a second, depending on the machine.
/// A render is a request, and paying that per request would make the second
/// image as slow as the first for no reason.
fn fonts() -> Result<Arc<usvg::fontdb::Database>, CoreError> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let db = DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        if let Some(family) = first_available(&db, &SANS_CANDIDATES) {
            db.set_sans_serif_family(family);
        }
        if let Some(family) = first_available(&db, &MONO_CANDIDATES) {
            db.set_monospace_family(family);
        }
        Arc::new(db)
    });
    if db.is_empty() {
        // The one case where refusing beats rendering: every label in this
        // image would be missing, and the picture would look finished.
        return Err(CoreError::Request(
            "no fonts are installed on this machine, so a PNG of this graph would carry \
             no labels at all. Render `--format svg` instead — the SVG carries its text \
             as text and picks up fonts wherever it is opened."
                .to_string(),
        ));
    }
    Ok(Arc::clone(db))
}

fn first_available(db: &usvg::fontdb::Database, candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|name| {
            db.query(&usvg::fontdb::Query {
                families: &[usvg::fontdb::Family::Name(name)],
                ..Default::default()
            })
            .is_some()
        })
        .map(|name| (*name).to_string())
}

/// Rasterise `document` at exactly `width` x `height`.
///
/// Exactly, not "about": the emitter writes a `viewBox` matching those numbers,
/// so the transform is the identity and a caller who asked for 1600x1000 gets
/// 1600x1000. A PNG whose dimensions were a function of the SVG's internal
/// units would make `--width` a suggestion.
pub fn to_png(document: &str, width: u32, height: u32) -> Result<Vec<u8>, CoreError> {
    let mut options = usvg::Options {
        fontdb: fonts()?,
        ..Default::default()
    };
    // Resolving `sans-serif` needs the *configured* family, but a `<text>` that
    // named nothing at all would fall back to this. Same family, so the two
    // paths cannot disagree.
    options.font_family = options
        .fontdb
        .family_name(&usvg::fontdb::Family::SansSerif)
        .to_string();

    let tree = usvg::Tree::from_str(document, &options).map_err(|err| {
        // Reaching here means this crate emitted invalid SVG, which is a bug
        // here and not in the caller's request — so it says so.
        CoreError::Request(format!(
            "the emitted SVG did not parse, which is a defect in kglite-visual's own \
             emitter rather than in this request: {err}"
        ))
    })?;

    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or_else(|| {
        CoreError::Request(format!(
            "a {width}x{height} pixmap could not be allocated for this render"
        ))
    })?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|err| CoreError::Request(format!("the PNG encoder failed: {err}")))
}
