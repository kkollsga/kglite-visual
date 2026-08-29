//! P4's server half: what the server spends before a byte reaches the wire.
//!
//! Run it in **release** and nothing else (`R11`); a debug number here is not
//! evidence. The harness script that drives it lives in `dev-docs/bench/
//! scripts/` and passes the graphs it generated.
//!
//! ```text
//! cargo run --release -p kglite-visual-core --example bench_server -- <graph.kgl> [repeats]
//! ```
//!
//! **Client and server are measured separately** (performance protocol §4), and
//! this is the server side: it holds a `Session` in-process and times the
//! compute and the encode, with no HTTP, no WebSocket and no browser in the
//! number. `capture.py` measures the same expansion a second way, over the
//! running CLI's JSON twin — the independent instrument §9 asks for, and
//! independent in the way that matters: a bug in one cannot be a bug in the
//! other, because one of them has a socket in it and the other does not.
//!
//! **The statistic is named per cell**, in the JSON this prints:
//! - `*_first` — *first event*. Loading a `.kgl` and opening a session happen
//!   once per open, and a repeat is a different question; the repeat is
//!   reported separately as its own median so both are on the record.
//! - expansion compute, encode, load, chunk sweep, control loops → *median*
//!   over `repeats`, with p95, min and max alongside, because a 30x+
//!   median-to-max spread on a deterministic operation is a rare expensive
//!   branch and therefore a finding rather than noise (§9).
//! - payload bytes, frame counts, node and edge counts → *exact*. They are
//!   deterministic, and a bound whose number is approximate is not a bound.
//!
//! **Two control cells**, per `R11`'s corollary, re-justified for this capture:
//! - `control_checksum` — an FNV-1a pass over a fixed 16 MiB buffer this file
//!   fills itself. It touches neither kglite nor the renderer, so no dependency
//!   bump can move it silently; only the machine can.
//! - `control_encode` — `ResponseEncoder` over a fixed 1 M-element `f32` array
//!   (4 MB, several chunks at every target in the sweep). Renderer-independent
//!   and graph-independent, so it anchors the encode cells without being able
//!   to move with them for a shared reason. It *is* ours to break, which is the
//!   point: it is the cell that says "the encoder changed" rather than "the
//!   machine drifted".
//!
//! Both run **before and after** the graph work, and both report their own
//! distribution, so the margin between a control's value and the capture's
//! scatter is readable off the output rather than asserted here. A reader
//! comparing two runs compares the controls first.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kglite_visual_core::protocol::{MessageType, ResponseEncoder, CHUNK_TARGET_BYTES};
use kglite_visual_core::request::{EdgeDirection, ExpandRequest, Request};
use kglite_visual_core::session::{response_frames, Response};
use kglite_visual_core::{load_graph, GraphSource, Session};

/// Slice sizes asked for, in nodes. The middle one is the response bound.
const SLICE_SIZES: [u32; 3] = [1_000, 5_000, 20_000];

/// Chunk targets swept, in bytes — D4's whole window, ends included.
const CHUNK_TARGETS: [usize; 3] = [256 * 1024, 512 * 1024, 1024 * 1024];

/// Bytes in the checksum control's buffer. Sized so the pass takes tens of
/// milliseconds: a control whose value sits close to the capture's own scatter
/// cannot show a 2x margin over it, and a control without that margin reports
/// noise as drift.
const CONTROL_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Elements in the encode control's array — 1 M `f32` is 4 MB, which is
/// several chunks at every target in the sweep.
const CONTROL_F32_LEN: usize = 1_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("usage: bench_server <graph.kgl> [repeats]")?,
    );
    let repeats: usize = match args.next() {
        Some(n) => n.parse()?,
        None => 21,
    };

    let mut out = Report::new();
    out.text("graph", path.display().to_string());
    out.number("repeats", repeats as f64);
    out.text(
        "profile",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
        .into(),
    );

    // ── controls, before anything else touches the machine ──────────────
    controls(&mut out, repeats, "control_pre");

    // ── open: load, then meta-graph. Separately, because they have
    //    different remedies and a single number cannot say which moved ───
    let load_start = Instant::now();
    let graph = load_graph(GraphSource::Path(&path))?;
    out.first_event("load_file_ms_first", ms(load_start.elapsed()));
    let open_start = Instant::now();
    let session = Session::open(graph.clone(), path.display().to_string());
    out.first_event("session_open_ms_first", ms(open_start.elapsed()));

    let info = session.info();
    out.exact("node_count", info.stats.node_count as f64);
    out.exact("edge_count", info.stats.edge_count as f64);
    out.text("tier", format!("{:?}", session.meta_graph().meta.tier));

    // Repeats of each half. `load_file` re-reads the `.kgl` every time, so its
    // repeat distribution is the page-cache-warm cost a second `open` pays;
    // `Session::open` over an already-loaded graph is the meta-graph compute
    // and nothing else, which is the O(#types) claim the entry screen rests on.
    let loads = samples(repeats, || {
        let g = load_graph(GraphSource::Path(&path)).expect("the graph loaded once");
        std::hint::black_box(Arc::strong_count(&g));
    });
    out.median("load_file_ms", &loads);
    let opens = samples(repeats, || {
        let s = Session::open(graph.clone(), "bench");
        std::hint::black_box(s.meta_graph().meta.nodes.len());
    });
    out.median("meta_graph_compute_ms", &opens);

    // Serving it again is a cache hit by construction — the meta-graph is
    // computed inside `open` and held. If this ever approached the compute
    // cell, every page reload would be paying for the entry screen twice.
    let meta_repeat = samples(repeats, || {
        let frames = session.meta_graph_frames();
        std::hint::black_box(frames.len());
    });
    out.median("meta_graph_frames_ms", &meta_repeat);
    let meta_bytes: usize = session.meta_graph_frames().iter().map(Vec::len).sum();
    out.exact("meta_graph_bytes", meta_bytes as f64);

    // ── expansion at each slice size, compute and encode separately ─────
    let person_slot = session
        .slot_of_type("Person")
        .ok_or("this graph has no Person type; the scale fixtures do")?;

    for limit in SLICE_SIZES {
        let request = Request::Expand(ExpandRequest {
            slot: person_slot,
            relationship: Some("KNOWS".into()),
            direction: EdgeDirection::Out,
            limit: Some(limit),
        });

        // A fresh session per repeat, built OUTSIDE the timed region: an
        // expansion appends slots, so a second `handle` on the same session
        // would answer about a view the first one already populated — and
        // building the session inside the clock would fold the meta-graph
        // compute into a cell named for the walk.
        let mut compute = Vec::with_capacity(repeats);
        for _ in 0..repeats {
            let session = Session::open(graph.clone(), "bench");
            let start = Instant::now();
            let response = session.handle(&request).expect("expansion is valid here");
            compute.push(ms(start.elapsed()));
            std::hint::black_box(&response);
        }

        let fresh = Session::open(graph.clone(), path.display().to_string());
        let response = fresh.handle(&request)?;
        let Response::Slice(slice) = &response else {
            return Err("expand did not answer with a slice".into());
        };
        let encode = samples(repeats, || {
            let frames = response_frames(&response);
            std::hint::black_box(frames.len());
        });
        let frames = response_frames(&response);
        let bytes: usize = frames.iter().map(Vec::len).sum();

        let cell = format!("expand_{limit}");
        out.median(&format!("{cell}_compute_ms"), &compute);
        out.median(&format!("{cell}_encode_ms"), &encode);
        out.exact(
            &format!("{cell}_nodes_returned"),
            slice.meta.bound.returned as f64,
        );
        out.exact(
            &format!("{cell}_nodes_total"),
            slice.meta.bound.total as f64,
        );
        out.exact(
            &format!("{cell}_truncated"),
            f64::from(slice.meta.bound.truncated),
        );
        out.exact(&format!("{cell}_links"), (slice.links.len() / 2) as f64);
        out.exact(&format!("{cell}_payload_bytes"), bytes as f64);
        out.exact(&format!("{cell}_frames"), frames.len() as f64);

        // The two array payloads on their own — the only part of a slice the
        // chunk target can act on, since JSON metadata is never chunked. Kept
        // as its own exact cell because it is what decides whether the sweep
        // below is measuring anything at this size.
        out.exact(
            &format!("{cell}_array_bytes"),
            ((slice.points.len() + slice.links.len()) * 4) as f64,
        );
        chunk_sweep(&mut out, &cell, &slice.points, &slice.links, repeats);
    }

    // ── the sweep on an array large enough to chunk at every target ─────
    //    A slice at the response bound does not reach one chunk (see the
    //    `*_array_bytes` cells), so the production numbers above cannot tell
    //    the three targets apart. This one can, and it is the same encoder.
    let saturating: Vec<f32> = (0..CONTROL_F32_LEN).map(|i| i as f32 * 0.5).collect();
    out.exact(
        "chunk_saturating_array_bytes",
        (saturating.len() * 4) as f64,
    );
    chunk_sweep(&mut out, "chunk_saturating", &saturating, &[], repeats);

    // ── controls again, after the load: the drift meter needs both ends ─
    controls(&mut out, repeats, "control_post");

    println!("{}", out.finish());
    Ok(())
}

/// The two control cells, measured under whatever load the machine has.
fn controls(out: &mut Report, repeats: usize, prefix: &str) {
    let buffer: Vec<u8> = (0..CONTROL_BUFFER_BYTES).map(|i| (i % 251) as u8).collect();
    let checksum = samples(repeats, || {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in &buffer {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        std::hint::black_box(hash);
    });
    out.median(&format!("{prefix}_checksum_ms"), &checksum);

    let values: Vec<f32> = (0..CONTROL_F32_LEN).map(|i| i as f32 * 0.5).collect();
    let encode = samples(repeats, || {
        let mut enc = ResponseEncoder::new();
        enc.push_f32(MessageType::Points, &values);
        std::hint::black_box(enc.finish().len());
    });
    out.median(&format!("{prefix}_encode_ms"), &encode);
    out.text(
        &format!("{prefix}_encode_chunk_bytes"),
        CHUNK_TARGET_BYTES.to_string(),
    );
}

/// Encode one slice's arrays at every chunk target in D4's window.
///
/// Reports the frame count as well as the time, because the frame count is what
/// says whether the target was reached at all: below one chunk's worth the
/// three targets are the same single frame and the timing difference between
/// them is not about chunking.
fn chunk_sweep(out: &mut Report, cell: &str, points: &[f32], links: &[f32], repeats: usize) {
    for target in CHUNK_TARGETS {
        let sweep = samples(repeats, || {
            let mut enc = ResponseEncoder::new();
            enc.push_f32_chunked(MessageType::Points, points, target);
            enc.push_f32_chunked(MessageType::Links, links, target);
            std::hint::black_box(enc.finish().len());
        });
        let mut enc = ResponseEncoder::new();
        enc.push_f32_chunked(MessageType::Points, points, target);
        enc.push_f32_chunked(MessageType::Links, links, target);
        let swept = enc.finish();
        let kb = target / 1024;
        out.median(&format!("{cell}_chunk{kb}k_ms"), &sweep);
        out.exact(&format!("{cell}_chunk{kb}k_frames"), swept.len() as f64);
        out.exact(
            &format!("{cell}_chunk{kb}k_bytes"),
            swept.iter().map(Vec::len).sum::<usize>() as f64,
        );
    }
}

/// Run `body` `n` times, returning every duration in milliseconds.
///
/// No warm-up discarded and no outlier trimmed: the whole distribution is
/// reported, because its shape is a diagnostic (§9) and a harness that averages
/// it away cannot see the rare expensive branch it was supposed to find.
fn samples(n: usize, mut body: impl FnMut()) -> Vec<f64> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let start = Instant::now();
        body();
        out.push(ms(start.elapsed()));
    }
    out
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx]
}

/// A JSON object, built in insertion order, every value tagged with the
/// statistic it is. A column of bare numbers whose statistic is implicit is a
/// column that will be compared wrongly.
struct Report {
    rows: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Self { rows: Vec::new() }
    }

    fn text(&mut self, key: &str, value: String) {
        self.rows.push(format!(
            "  {:?}: {{\"statistic\": \"exact\", \"value\": {value:?}}}",
            key
        ));
    }

    fn number(&mut self, key: &str, value: f64) {
        self.exact(key, value);
    }

    fn exact(&mut self, key: &str, value: f64) {
        self.rows.push(format!(
            "  {key:?}: {{\"statistic\": \"exact\", \"value\": {value}}}"
        ));
    }

    fn first_event(&mut self, key: &str, value: f64) {
        self.rows.push(format!(
            "  {key:?}: {{\"statistic\": \"first-event\", \"value\": {value:.4}}}"
        ));
    }

    fn median(&mut self, key: &str, values: &[f64]) {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let median = quantile(&sorted, 0.5);
        let max = sorted.last().copied().unwrap_or(f64::NAN);
        self.rows.push(format!(
            "  {key:?}: {{\"statistic\": \"median\", \"value\": {median:.4}, \
             \"p95\": {:.4}, \"min\": {:.4}, \"max\": {max:.4}, \
             \"spread_ratio\": {:.2}, \"n\": {}}}",
            quantile(&sorted, 0.95),
            sorted[0],
            if median > 0.0 { max / median } else { f64::NAN },
            sorted.len()
        ));
    }

    fn finish(self) -> String {
        format!("{{\n{}\n}}", self.rows.join(",\n"))
    }
}
