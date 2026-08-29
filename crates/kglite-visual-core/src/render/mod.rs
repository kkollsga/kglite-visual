//! Cypher in, image out (plan D13).
//!
//! One core capability, thin faces — D4's rule applied to a second surface. A
//! request names a *source* (the meta-graph, a Cypher result, a bounded
//! expansion), this builds that slice **through the pipeline that already
//! exists**, lays it out deterministically, and emits an image. The CLI
//! subcommand and `POST /api/render` are two ways to call this function; they
//! contain no drawing logic and no bound of their own.
//!
//! **Nothing here reaches around the bound.** The slice is produced by
//! [`crate::session::Session`], so `expand::effective_bound` and the query row
//! bound apply unchanged, and the same `{returned, total, truncated}` metadata
//! comes back. What is new is that the metadata is *drawn into the image*: an
//! agent handed a picture has no status bar to read, so a truncated render that
//! did not say so would be the D5 failure in its purest form.
//!
//! **A render of a *request* never touches the live view.** It opens a private
//! session over the same `Arc<DirGraph>` — the graph is `Send + Sync` and
//! read-only, so this costs a meta-graph computation and no I/O. The
//! alternative, running the request against the caller's session, would make
//! `POST /api/render` mutate the slot space of whatever browser tab happens to
//! be attached: an image request is a question, and a question that moves the
//! user's screen is a bug.
//!
//! **[`RenderSource::LiveView`] is the one that reads that session** (P10), and
//! it still does not move it: it draws the slots that are already there. Same
//! content as the user's screen, different geometry — the layout below is a
//! seeded Fruchterman-Reingold and the user's is cosmos.gl's GPU simulation, so
//! the two will never coincide by accident. [`crate::session::GEOMETRY_CAVEAT`]
//! is the sentence every face repeats about that, and it is one constant so it
//! cannot be repeated differently.

pub mod encoding;
pub mod labels;
pub mod layout;
pub mod raster;
pub mod svg;

use std::sync::Arc;

use kglite::api::DirGraph;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::meta_graph::MetaGraphResponse;
use crate::query::QueryConfig;
use crate::request::{CypherRequest, EdgeDirection, ExpandRequest, Request};
use crate::session::{GraphSlice, Response, Session};
use crate::view::{SliceKind, SlotEntry};

pub use encoding::Theme;

/// Canvas defaults, in pixels.
///
/// Wide rather than square: the label chips are horizontal, so width is what
/// buys legibility, and 16:10 is the shape a graph lands in when it is pasted
/// anywhere.
///
/// **The size is a density argument, not a taste one, and it was measured on
/// the real graph.** A label occupies a 130x30 px cell
/// (`labels::CELL_WIDTH`), so `n` labels need about `n` cells laid out with
/// nothing overlapping: at sodir's 98 types that is a mean spacing of
/// `sqrt(area / (0.866 n))`. 1600x1000 gives 137 px — *exactly* one cell width,
/// which in practice left a dozen names stacked on each other in the hub
/// cluster. 2000x1250 gives 171 px, and the same render comes back with every
/// type readable. Anything much larger is a file nobody wants to scroll, and
/// `--width` / `--height` are there for the graph that needs it.
pub const DEFAULT_WIDTH: u32 = 2000;
pub const DEFAULT_HEIGHT: u32 = 1250;

/// Smallest canvas that can hold the status block and a graph. Below this the
/// chrome is larger than the picture.
const MIN_DIMENSION: u32 = 200;
/// Largest canvas. A PNG is `width * height * 4` bytes in memory before it is
/// encoded, so this is a real allocation ceiling, not a style rule.
const MAX_DIMENSION: u32 = 8_000;

/// What the caller wants a picture of.
///
/// Deserialize-only, like [`crate::request::Request`]: these are things a
/// caller *asks for*, and the shape a `curl` body writes is the whole contract.
/// Deliberately not a `ts-rs` type either — the frontend does not consume this
/// endpoint; agents and `curl` do, the same argument `DescribeResponse` makes.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum RenderSource {
    /// The type-level meta-graph — the entry screen, as an image.
    Meta,
    /// A Cypher result, drawn as a graph. Runs through the same path the app's
    /// "show in graph" checkbox uses, so the bound and the row limit are the
    /// same ones.
    Cypher(CypherRequest),
    /// A bounded neighbourhood expansion from a named type.
    ///
    /// By *name*, not by slot: a render has no session for a caller to have
    /// learned a slot from. The name is resolved against the private session's
    /// own meta-graph.
    Expand(ExpandSource),
    /// **The live view itself** — whatever is on the shared screen right now,
    /// laid out server-side (P10).
    ///
    /// The only source that reads the caller's session rather than a private
    /// one, and the only one whose answer changes without the request changing.
    /// It draws; it never mutates. Reachable only through
    /// [`render_for`] — [`render`] has no session and says so.
    LiveView,
}

/// The expansion half of a render request.
#[derive(Debug, Clone, Deserialize)]
pub struct ExpandSource {
    /// The type to expand out of.
    pub node_type: String,
    /// Relationship type to walk. `None` walks every type.
    #[serde(default)]
    pub relationship: Option<String>,
    #[serde(default)]
    pub direction: EdgeDirection,
    /// Nodes wanted. Clamped to the core ceiling; see [`crate::expand`].
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Which bytes come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderFormat {
    /// The native emitter: text is text, the viewBox scales, nothing is
    /// fetched. Default because it is the format that stays readable.
    #[default]
    Svg,
    /// Rasterised from the same SVG by resvg. Derived, never authored — see
    /// [`raster`]. No JPEG, ever: it is the wrong codec for line art (D13).
    Png,
}

impl RenderFormat {
    pub fn content_type(self) -> &'static str {
        match self {
            RenderFormat::Svg => "image/svg+xml",
            RenderFormat::Png => "image/png",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            RenderFormat::Svg => "svg",
            RenderFormat::Png => "png",
        }
    }
}

/// One render.
#[derive(Debug, Clone, Deserialize)]
pub struct RenderRequest {
    pub source: RenderSource,
    #[serde(default)]
    pub format: RenderFormat,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    /// Reaches the layout's initial placement only. Two seeds are two starting
    /// points of one deterministic process, not two samples of a random one.
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub theme: Theme,
}

fn default_width() -> u32 {
    DEFAULT_WIDTH
}

fn default_height() -> u32 {
    DEFAULT_HEIGHT
}

impl RenderRequest {
    /// A meta-graph render at the defaults — the shape most callers want.
    pub fn meta() -> Self {
        Self {
            source: RenderSource::Meta,
            format: RenderFormat::default(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            seed: 0,
            theme: Theme::default(),
        }
    }
}

/// The image, plus what an agent needs to know about it without opening it.
///
/// The counts and the truncation flag are the part a machine reads: an image is
/// not introspectable, so a render that came back clipped has to say so in a
/// field as well as in the picture.
#[derive(Debug, Clone, Serialize)]
pub struct Rendered {
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub format: RenderFormat,
    pub width: u32,
    pub height: u32,
    /// Nodes drawn.
    pub nodes: u32,
    /// Links drawn.
    pub links: u32,
    /// True when any bound clipped this answer.
    pub truncated: bool,
    /// The banner text drawn into the image, verbatim — the same words the app
    /// shows for the same response.
    pub banners: Vec<String>,
}

/// One node, ready to draw.
#[derive(Debug, Clone)]
pub(crate) struct SceneNode {
    pub slot: u32,
    pub text: String,
    pub weight: u64,
    pub radius: f64,
    pub color: encoding::Rgba,
    pub badges: Vec<String>,
    pub dimmed: bool,
}

/// One link, ready to draw.
#[derive(Debug, Clone)]
pub(crate) struct SceneLink {
    pub source: usize,
    pub target: usize,
    pub width: f64,
}

/// Everything the emitter needs and nothing it does not.
#[derive(Debug, Clone)]
pub(crate) struct Scene {
    pub nodes: Vec<SceneNode>,
    pub links: Vec<SceneLink>,
    /// Status lines, drawn top-left in the same order the app's status block
    /// lists them.
    pub status: Vec<String>,
    /// Truncation banners, drawn under the status block in the warn colour.
    pub banners: Vec<String>,
    /// True while the view is nothing but the type-level meta-graph — the
    /// condition `isMetaGraphOnly` tests in `frontend/src/main.ts`, and the one
    /// that decides whether a label that loses its cell is nudged or dropped.
    pub place_all_labels: bool,
}

/// Build the slice, lay it out, and emit the image.
///
/// `source_name` is what the picture calls the graph — a path, or whatever the
/// caller opened it as.
pub fn render(
    graph: &Arc<DirGraph>,
    source_name: &str,
    config: QueryConfig,
    request: &RenderRequest,
) -> Result<Rendered, CoreError> {
    if matches!(request.source, RenderSource::LiveView) {
        return Err(CoreError::Request(
            "`live-view` draws the view of a RUNNING session, and this entry point \
             opens a private one that has no live view to draw. Ask the running \
             server instead: `POST /api/render` on its URL, or the MCP `render` tool."
                .to_string(),
        ));
    }
    // The private session: see the module doc. `Session::open_with` computes the
    // meta-graph once, which is O(#types) plus a read of the connectivity cache
    // the `.kgl` already carries.
    let session = Session::open_with(Arc::clone(graph), source_name.to_string(), config);
    draw(&session, request)
}

/// Draw a request against a running server's session.
///
/// **The one place the privacy rule is decided.** `live-view` reads that
/// session; every other source gets a private one, so a `POST /api/render` for
/// a Cypher result cannot append slots to the screen a human is watching. A
/// second call site making this choice again would eventually make it
/// differently.
pub fn render_for(session: &Session, request: &RenderRequest) -> Result<Rendered, CoreError> {
    match request.source {
        RenderSource::LiveView => draw(session, request),
        RenderSource::Meta | RenderSource::Cypher(_) | RenderSource::Expand(_) => render(
            session.graph(),
            &session.info().graph,
            session.config(),
            request,
        ),
    }
}

fn draw(session: &Session, request: &RenderRequest) -> Result<Rendered, CoreError> {
    let width = check_dimension(request.width, "width")?;
    let height = check_dimension(request.height, "height")?;

    let scene = build_scene(session, request)?;
    if scene.nodes.is_empty() {
        // An empty canvas is the worst possible answer here: it is
        // indistinguishable from a rendering failure, and an agent handed one
        // has no way to tell "nothing matched" from "the renderer broke". The
        // query that produced this project's own first zero-node render was a
        // typo in a relationship name, and the picture said nothing at all.
        return Err(CoreError::Request(
            "this request selected no nodes, so there is nothing to draw. \
             Check the query or the type name against `GET /api/meta-graph`."
                .to_string(),
        ));
    }

    let nodes: Vec<layout::LayoutNode> = scene
        .nodes
        .iter()
        .map(|n| layout::LayoutNode { radius: n.radius })
        .collect();
    let links: Vec<(usize, usize)> = scene.links.iter().map(|l| (l.source, l.target)).collect();
    let positions = layout::run(
        &nodes,
        &links,
        f64::from(width),
        f64::from(height),
        request.seed,
        // The layout is told about the status block so nothing is laid out
        // underneath it; the emitter owns that rectangle's geometry.
        svg::status_block_height(scene.status.len() + scene.banners.len()),
    )?;

    let document = svg::emit(&scene, &positions, width, height, request.theme);
    let bytes = match request.format {
        RenderFormat::Svg => document.into_bytes(),
        RenderFormat::Png => raster::to_png(&document, width, height)?,
    };

    Ok(Rendered {
        bytes,
        format: request.format,
        width,
        height,
        nodes: scene.nodes.len() as u32,
        links: scene.links.len() as u32,
        truncated: !scene.banners.is_empty(),
        banners: scene.banners,
    })
}

fn check_dimension(value: u32, name: &str) -> Result<u32, CoreError> {
    if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&value) {
        return Err(CoreError::Request(format!(
            "{name} must be between {MIN_DIMENSION} and {MAX_DIMENSION} pixels; got {value}"
        )));
    }
    Ok(value)
}

fn build_scene(session: &Session, request: &RenderRequest) -> Result<Scene, CoreError> {
    match &request.source {
        RenderSource::Meta => Ok(meta_scene(session.meta_graph(), session)),
        RenderSource::Cypher(cypher) => {
            let mut cypher = cypher.clone();
            // A render is a picture of a graph, so the graph half of the result
            // is what it needs. Forcing the flag rather than refusing a caller
            // who left it off: `as_graph: false` would produce a table, and a
            // table is not something this function can draw.
            cypher.as_graph = true;
            let Response::Slice(slice) = session.handle(&Request::Cypher(cypher))? else {
                unreachable!("as_graph was forced above, and that path answers with a slice");
            };
            Ok(slice_scene(session, &slice))
        }
        RenderSource::Expand(expand) => {
            let slot = session.slot_of_type(&expand.node_type).ok_or_else(|| {
                CoreError::Request(format!(
                    "this graph has no type named {:?} on its meta-graph; \
                     the types it does have are listed by `GET /api/meta-graph`",
                    expand.node_type
                ))
            })?;
            let Response::Slice(slice) = session.handle(&Request::Expand(ExpandRequest {
                slot,
                relationship: expand.relationship.clone(),
                direction: expand.direction,
                limit: expand.limit,
            }))?
            else {
                unreachable!("an expand request answers with a slice");
            };
            Ok(slice_scene(session, &slice))
        }
        RenderSource::LiveView => Ok(live_scene(session)),
    }
}

/// Everything currently in a running session's slot space, as a scene (P10).
///
/// Unlike [`slice_scene`], this keeps the **type nodes**: they are on the
/// user's screen, and an image of the live view that dropped them would not be
/// a picture of what the user is looking at, which is the entire request. That
/// is also why it is a third builder rather than a flag on the second — the two
/// answer different questions, and the "drop the meta-graph" decision is right
/// for exactly one of them.
///
/// **Content, not geometry.** The positions here come from this module's own
/// seeded layout, never from the client. See
/// [`crate::session::GEOMETRY_CAVEAT`].
fn live_scene(session: &Session) -> Scene {
    let view = session.view_read();
    let meta_by_name: std::collections::HashMap<&str, &crate::meta_graph::MetaTypeNode> = session
        .meta_graph()
        .meta
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();

    // The size ramp is relative to the largest type ON SCREEN, mirroring
    // `maxTypeCount` in `frontend/src/main.ts`: a view drilled into one small
    // type must not draw it as a dot because some other type it is no longer
    // showing is larger.
    let largest = view
        .live_entries()
        .filter_map(|(_, entry)| match entry {
            SlotEntry::Type { name } => meta_by_name.get(name.as_str()).map(|node| node.count),
            _ => None,
        })
        .max()
        .unwrap_or(1)
        .max(1);

    let mut nodes: Vec<SceneNode> = Vec::new();
    let mut index_of: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut instance_count = 0usize;
    for (slot, entry) in view.live_entries() {
        let node = match entry {
            SlotEntry::Type { name } => {
                let meta = meta_by_name.get(name.as_str());
                let count = meta.map(|m| m.count).unwrap_or(0);
                let capabilities = meta.map(|m| m.capabilities.clone()).unwrap_or_default();
                let supporting = meta.is_some_and(|m| m.supporting);
                SceneNode {
                    slot,
                    text: name.clone(),
                    weight: u64::from(count),
                    radius: encoding::type_radius(count, largest, supporting),
                    color: encoding::base_color(true, !capabilities.is_empty(), supporting),
                    badges: capabilities,
                    dimmed: supporting,
                }
            }
            SlotEntry::Node {
                node_id,
                node_type,
                title,
            } => {
                instance_count += 1;
                SceneNode {
                    slot,
                    // The same display fallback the client uses
                    // (`View.applySlice` in `frontend/src/view.ts`): the stored
                    // title is the empty string the graph actually held, and
                    // the substitute is made here rather than persisted.
                    text: if title.is_empty() {
                        format!("{node_type} {node_id}")
                    } else {
                        title.clone()
                    },
                    weight: 1,
                    radius: encoding::INSTANCE_RADIUS_PX,
                    color: encoding::base_color(false, false, false),
                    badges: Vec::new(),
                    dimmed: false,
                }
            }
            SlotEntry::Tombstone => unreachable!("live_entries skips tombstones"),
        };
        index_of.insert(slot, nodes.len());
        nodes.push(node);
    }

    // Meta links carry a count and take the width ramp; instance links stand
    // for one edge each and take the floor. Same two rules as `linkWidths` in
    // `frontend/src/main.ts`, applied to one mixed list because the live view
    // is the one place both kinds are on screen together.
    let mut pair_edges: std::collections::HashMap<(u32, u32), u64> =
        std::collections::HashMap::new();
    for edge in view.edges().iter().filter(|e| e.meta) {
        *pair_edges
            .entry(pair_key(edge.source_slot, edge.target_slot))
            .or_insert(0) += u64::from(meta_edge_count(session, edge));
    }
    let heaviest = pair_edges.values().copied().max().unwrap_or(1).max(1);

    let links: Vec<SceneLink> = view
        .edges()
        .iter()
        .filter_map(|edge| {
            let width = if edge.meta {
                let count = pair_edges
                    .get(&pair_key(edge.source_slot, edge.target_slot))
                    .copied()
                    .unwrap_or(0);
                encoding::link_width(clamp_u32(count), clamp_u32(heaviest))
            } else {
                encoding::LINK_MIN_PX
            };
            Some(SceneLink {
                source: *index_of.get(&edge.source_slot)?,
                target: *index_of.get(&edge.target_slot)?,
                width,
            })
        })
        .collect();

    let state = session.view_state();
    let mut status = status_lines(session, nodes.len(), links.len(), "slots drawn");
    if state.tombstone_count > 0 {
        status.push(format!(
            "{} collapsed",
            encoding::group_thousands(u64::from(state.tombstone_count))
        ));
    }

    Scene {
        status,
        nodes,
        links,
        // The banner the app is showing right now, verbatim — not one recomputed
        // here. An image of the live view that disagreed with the status bar
        // beside it would be worse than one with no banner at all.
        banners: state
            .last_slice
            .and_then(|last| last.banner)
            .into_iter()
            .collect(),
        // The same condition `isMetaGraphOnly` tests in `frontend/src/main.ts`:
        // a view that is nothing but type nodes is a picture OF its labels.
        place_all_labels: instance_count == 0,
    }
}

/// The edge count a meta link stands for, from the meta-graph it came out of.
///
/// The view's own `ViewEdge` does not carry it — the count is meta-graph
/// metadata, and the slot space deliberately holds identity rather than
/// weight.
fn meta_edge_count(session: &Session, edge: &crate::view::ViewEdge) -> u32 {
    session
        .meta_graph()
        .meta
        .edges
        .iter()
        .find(|e| {
            e.source_slot == edge.source_slot
                && e.target_slot == edge.target_slot
                && e.name == edge.name
        })
        .map(|e| e.count)
        .unwrap_or(0)
}

/// The entry screen, as a scene.
fn meta_scene(meta: &MetaGraphResponse, session: &Session) -> Scene {
    let largest = meta.meta.nodes.iter().map(|n| n.count).max().unwrap_or(1);
    let index_of: std::collections::HashMap<u32, usize> = meta
        .meta
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.slot, i))
        .collect();

    let nodes: Vec<SceneNode> = meta
        .meta
        .nodes
        .iter()
        .map(|n| SceneNode {
            slot: n.slot,
            text: n.name.clone(),
            weight: u64::from(n.count),
            radius: encoding::type_radius(n.count, largest, n.supporting),
            color: encoding::base_color(true, !n.capabilities.is_empty(), n.supporting),
            badges: n.capabilities.clone(),
            dimmed: n.supporting,
        })
        .collect();

    // Two type nodes can be joined by several relationship types, and the width
    // channel answers "how much traffic is there between these two", so they
    // add. Mirrors `View.typeLinkWeight` in `frontend/src/view.ts`.
    let mut pair_edges: std::collections::HashMap<(u32, u32), u64> =
        std::collections::HashMap::new();
    for edge in &meta.meta.edges {
        let key = pair_key(edge.source_slot, edge.target_slot);
        *pair_edges.entry(key).or_insert(0) += u64::from(edge.count);
    }
    let heaviest = pair_edges.values().copied().max().unwrap_or(1).max(1);

    let links: Vec<SceneLink> = meta
        .meta
        .edges
        .iter()
        .filter_map(|edge| {
            let count = pair_edges
                .get(&pair_key(edge.source_slot, edge.target_slot))
                .copied()
                .unwrap_or(0);
            Some(SceneLink {
                source: *index_of.get(&edge.source_slot)?,
                target: *index_of.get(&edge.target_slot)?,
                width: encoding::link_width(clamp_u32(count), clamp_u32(heaviest)),
            })
        })
        .collect();

    // Mirrors the meta-graph half of `renderStatus` in `frontend/src/main.ts`,
    // wording included — including that those two banners are NOT thousands-
    // grouped, unlike the slice banner below.
    let mut banners = Vec::new();
    if meta.meta.node_bound.truncated {
        banners.push(format!(
            "showing {} of {} types",
            meta.meta.node_bound.returned, meta.meta.node_bound.total
        ));
    }
    if meta.meta.edge_bound.truncated {
        banners.push(format!(
            "showing {} of {} relationships",
            meta.meta.edge_bound.returned, meta.meta.edge_bound.total
        ));
    }

    Scene {
        status: status_lines(session, nodes.len(), links.len(), "types"),
        nodes,
        links,
        banners,
        // The meta-graph IS its labels: a type node with no name on it is a dot.
        place_all_labels: true,
    }
}

/// A query result or an expansion, as a scene.
///
/// **Instance nodes only.** The private session's view also holds the
/// meta-graph's type nodes, because that is what a session is; drawing them
/// alongside a 200-node neighbourhood would put ninety-eight unconnected
/// circles around the thing the caller actually asked for. The app does not
/// have this choice — it has one continuous view a user navigates — and an
/// image is a snapshot of one question.
fn slice_scene(session: &Session, slice: &GraphSlice) -> Scene {
    let index_of: std::collections::HashMap<u32, usize> = slice
        .meta
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.slot, i))
        .collect();

    let nodes: Vec<SceneNode> = slice
        .meta
        .nodes
        .iter()
        .map(|n| SceneNode {
            slot: n.slot,
            // A node with no title gets its type and id rather than a blank
            // label. Mirrors the display fallback in `View.applySlice`
            // (`frontend/src/view.ts`); the server still sends the empty string
            // it actually found.
            text: if n.title.is_empty() {
                format!("{} {}", n.node_type, n.node_id)
            } else {
                n.title.clone()
            },
            // Mirrors `weight: 1` for an instance slot in
            // `frontend/src/view.ts`: an instance carries no count, so the
            // label grid's tie-break falls through to the slot id.
            weight: 1,
            radius: encoding::INSTANCE_RADIUS_PX,
            color: encoding::base_color(false, false, false),
            badges: Vec::new(),
            dimmed: false,
        })
        .collect();

    let links: Vec<SceneLink> = slice
        .meta
        .edges
        .iter()
        .filter(|e| !e.meta)
        .filter_map(|e| {
            Some(SceneLink {
                source: *index_of.get(&e.source_slot)?,
                target: *index_of.get(&e.target_slot)?,
                // An instance link stands for exactly one edge, so there is no
                // count to encode and it takes the floor. Mirrors the
                // "not a meta link" branch of `linkWidths` in
                // `frontend/src/main.ts`.
                width: encoding::LINK_MIN_PX,
            })
        })
        .collect();

    let unit = if slice.meta.kind == SliceKind::Collapse {
        "collapsed"
    } else {
        "nodes"
    };
    let banner = encoding::truncation_banner(
        slice.meta.bound.truncated,
        slice.meta.bound.returned,
        slice.meta.bound.total,
        unit,
        Some((
            slice.meta.link_bound.truncated,
            slice.meta.link_bound.returned,
            slice.meta.link_bound.total,
        )),
    );

    Scene {
        status: status_lines(session, nodes.len(), links.len(), "nodes"),
        nodes,
        links,
        banners: banner.into_iter().collect(),
        // An instance slice keeps dropping: at the 5 000-node response bound
        // "every label" is not a picture at any density.
        place_all_labels: false,
    }
}

/// The status block, mirroring the informative half of `renderStatus` in
/// `frontend/src/main.ts` — minus the connection state, which an image has no
/// equivalent of.
fn status_lines(session: &Session, nodes: usize, links: usize, unit: &str) -> Vec<String> {
    let info = session.info();
    vec![
        info.graph.clone(),
        format!(
            "tier {} · {} nodes / {} edges",
            serde_json::to_string(&info.tier)
                .unwrap_or_default()
                .trim_matches('"'),
            encoding::group_thousands(u64::from(info.stats.node_count)),
            encoding::group_thousands(u64::from(info.stats.edge_count))
        ),
        format!(
            "{} {unit} / {} links drawn",
            encoding::group_thousands(nodes as u64),
            encoding::group_thousands(links as u64)
        ),
    ]
}

/// Order-insensitive slot pair — the meta-graph draws one line per pair and the
/// width channel is about the pair, not about a direction a line cannot show.
/// Mirrors `pairKey` in `frontend/src/view.ts`.
fn pair_key(a: u32, b: u32) -> (u32, u32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn clamp_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
