//! The embedded frontend.
//!
//! `rust-embed` bakes `frontend/dist` into the binary, so the product is one
//! executable with no install step (the tensorboard / marimo pattern).
//!
//! Two feature choices, both deliberate:
//! - **`debug-embed` always.** Without it a debug build reads assets off disk
//!   at runtime, so "works in dev, 404s in release" becomes a class of bug that
//!   only the packaged artifact can reveal — and the packaged artifact is the
//!   thing hardest to test.
//! - **`interpolate-folder-path`**, so the folder is
//!   `$CARGO_MANIFEST_DIR/../../frontend/dist` rather than a path relative to
//!   whatever directory cargo happened to be invoked from.

use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../frontend/dist"]
struct Assets;

/// Serve an embedded asset, falling back to `index.html`.
///
/// **A path with a file extension gets a real 404.** The usual SPA fallback
/// answers *everything* with `index.html`, which turns a missing script into a
/// 200 whose body is HTML — the browser then reports a MIME/parse error that
/// names neither the missing file nor the server. Extensionless paths are
/// routes and do get the app; `/assets/index-a1b2c3.js` either exists or is
/// honestly absent.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = Assets::get(path) {
        return asset_response(path, file.data.into_owned());
    }

    if has_extension(path) {
        return (
            StatusCode::NOT_FOUND,
            format!("no embedded asset at /{path}\n"),
        )
            .into_response();
    }

    match Assets::get("index.html") {
        Some(file) => asset_response("index.html", file.data.into_owned()),
        // Reachable only from a binary built against an empty dist/. Saying so
        // is the difference between a five-minute fix and an afternoon spent
        // in the server code.
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "no frontend bundle is embedded in this binary — it was built \
             without frontend/dist\n",
        )
            .into_response(),
    }
}

/// Only the last path segment can carry an extension; a dot in a directory
/// name (`/v1.2/settings`) must not turn a route into a 404.
fn has_extension(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
}

fn asset_response(path: &str, body: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path == "index.html" {
        // index.html is the one file whose *name* never changes, so a cached
        // copy pins the app to the hashed asset names it was built with. A
        // user who updates the binary would keep getting yesterday's app.
        "no-cache"
    } else {
        // Vite hashes every other emitted name, so the content at a given URL
        // is immutable by construction.
        "public, max-age=31536000, immutable"
    };

    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime.as_ref()).expect("a MIME type is a valid header value"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
        ],
        Body::from(body),
    )
        .into_response()
}

/// Whether any asset is embedded at all — the packaged-consumer question a
/// source-tree test structurally cannot answer.
pub fn embedded_file_count() -> usize {
    Assets::iter().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dot_in_a_directory_name_is_not_an_extension() {
        assert!(has_extension("assets/index-a1b2c3.js"));
        assert!(has_extension("favicon.ico"));
        assert!(!has_extension("graph/Person"));
        assert!(!has_extension("v1.2/settings"));
        assert!(!has_extension(""));
    }
}
