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
pub mod structure;
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
    /// Nodes the slice carried — **drawn plus folded**, so this stays the
    /// answer to "how big was the result" whether or not any fan was folded.
    pub nodes: u32,
    /// Links the slice carried — counted, like `nodes`, before any fan was
    /// folded, so the JSON and the status block drawn into the image agree.
    pub links: u32,
    /// Nodes folded into aggregate wedges and therefore NOT drawn individually
    /// (P11). `nodes - folded` is what a reader can count in the picture.
    pub folded: u32,
    /// Wall-clock milliseconds the layout pass took.
    ///
    /// Reported because the layout is now chosen per scene and the three
    /// kernels have different costs; a caller that suddenly waits seconds for
    /// an image needs the number that says which half is slow.
    pub layout_ms: f64,
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
    /// The node's type — what the hue is drawn from and what the ring layout
    /// groups a fan by. A type node's own name for a meta node; `None` only
    /// where the source could not say.
    pub node_type: Option<String>,
    /// Whether the count chip is drawn. See [`labels::LabelSpec::show_count`].
    pub show_count: bool,
    /// This node's label is placed before all others and never dropped. See
    /// [`labels::LabelSpec::pinned`].
    pub pinned: bool,
    /// This node is a folded fan standing for `Some(n)` nodes that are NOT in
    /// the picture — drawn as a wedge, never as a circle. See
    /// [`fold_leaf_fans`].
    pub aggregate: Option<u32>,
    /// This node is the one a radial layout is centred on: drawn larger, with a
    /// halo, and never thinned. Set by [`emphasise_centres`] once the plan is
    /// known, never by a scene builder — a builder cannot know which layout
    /// kernel the structure will choose.
    pub emphasis: bool,
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
    /// Node indices the *request* named as its origin.
    ///
    /// One entry is an ego request and gets a radial layout on the caller's
    /// word alone; more than one is an expansion from a whole type, which names
    /// no centre, and falls through to [`structure::plan`]. Empty for a request
    /// with no origin at all.
    pub seeds: Vec<usize>,
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

    let mut scene = build_scene(session, request)?;
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

    // Counted before the fold, because the status block inside the image is
    // also written before it: a JSON line reporting 556 links beside a picture
    // whose own banner says 748 makes a reader doubt both numbers. Both halves
    // answer "what did the slice carry"; `folded` is the one that says how
    // much of it a reader can count.
    let slice_nodes = scene.nodes.len() as u32;
    let slice_links = scene.links.len() as u32;
    let folded = fold_leaf_fans(&mut scene, f64::from(width) * f64::from(height));
    if folded > 0 {
        // Drawn INTO the image, beside the truncation banner and for the same
        // reason (D5): a picture with a fan glyph in it and nothing saying how
        // many nodes are behind the glyph is a picture that reads as complete.
        scene.status.push(format!(
            "{} folded into {} fans",
            encoding::group_thousands(u64::from(folded)),
            scene.nodes.iter().filter(|n| n.aggregate.is_some()).count()
        ));
    }

    let links: Vec<(usize, usize)> = scene.links.iter().map(|l| (l.source, l.target)).collect();
    let plan = structure::plan(scene.nodes.len(), &links, &scene.seeds);
    // Before the layout nodes are built, because emphasis changes a radius and
    // a ring is sized from the radii it has to hold.
    emphasise_centres(&mut scene, &plan, &links);

    let nodes: Vec<layout::LayoutNode> = scene
        .nodes
        .iter()
        .map(|n| layout::LayoutNode { radius: n.radius })
        .collect();
    // The layout is told about the status block so nothing is laid out
    // underneath it; the emitter owns that rectangle's geometry.
    let canvas = layout::Canvas {
        width: f64::from(width),
        height: f64::from(height),
        reserved_top: svg::status_block_height(scene.status.len() + scene.banners.len()),
    };
    let groups = arc_groups(&scene);
    let started = std::time::Instant::now();
    let positions = match plan {
        structure::Plan::Radial { seed } => layout::radial(&nodes, &links, seed, &groups, canvas)?,
        structure::Plan::Islands { community, count } => layout::islands(
            &nodes,
            &links,
            &community,
            count,
            &groups,
            canvas,
            request.seed,
        )?,
        structure::Plan::Force => layout::run(&nodes, &links, canvas, request.seed)?,
    };
    let layout_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let mut positions = positions;
    moor_folded_fans(&scene, &mut positions, f64::from(width), f64::from(height));

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
        nodes: slice_nodes,
        links: slice_links,
        folded,
        layout_ms,
        truncated: !scene.banners.is_empty(),
        banners: scene.banners,
    })
}

/// Gap between a folded fan's parent and the wedge standing for its children,
/// in pixels — long enough that the connector is a visible line and short
/// enough that the two read as one object.
const WEDGE_TETHER_PX: f64 = 26.0;

/// Move every folded fan next to the node it hangs off.
///
/// **A wedge that floats is a lie by omission** (P11 round 2). The glyph says
/// `Wellbore x 27`, and twenty-seven of something are attached to *what*? The
/// fold already pins the parent's label for that reason, and the layout already
/// draws the one link — but a general layout kernel treats the glyph as an
/// ordinary node, so it can settle a third of the frame away with its
/// connector lost among four hundred other lines. Nothing about the fold is
/// legible at that distance.
///
/// The direction is the one the layout chose (parent -> glyph, normalised); only
/// the distance is overridden, so the wedge still opens into whatever space the
/// kernel found for it. A glyph the layout happened to place on top of its
/// parent is pushed away from the picture's centre instead, which is where the
/// room is.
///
/// Runs after the layout rather than inside it because it is true of all three
/// kernels and depends on none of them: the constraint is "adjacent to a named
/// parent", and every kernel that produced a position has already answered the
/// question this corrects.
fn moor_folded_fans(scene: &Scene, positions: &mut layout::Positions, width: f64, height: f64) {
    if !scene.nodes.iter().any(|node| node.aggregate.is_some()) {
        return;
    }
    let (mut sum_x, mut sum_y) = (0.0f64, 0.0f64);
    for (x, y) in &positions.xy {
        sum_x += x;
        sum_y += y;
    }
    let divisor = positions.xy.len().max(1) as f64;
    let centre = (sum_x / divisor, sum_y / divisor);

    for index in 0..scene.nodes.len() {
        if scene.nodes[index].aggregate.is_none() {
            continue;
        }
        let parent = scene.links.iter().find_map(|link| {
            if link.source == index {
                Some(link.target)
            } else if link.target == index {
                Some(link.source)
            } else {
                None
            }
        });
        // A fold always produces exactly one parent link. A glyph with none is
        // not reachable today; leaving it where the layout put it is the only
        // honest answer, since there is no parent to sit beside.
        let (Some(parent), Some(&(px, py))) = (parent, parent.and_then(|p| positions.xy.get(p)))
        else {
            continue;
        };
        let (gx, gy) = positions.xy[index];
        let (mut dx, mut dy) = (gx - px, gy - py);
        let mut length = (dx * dx + dy * dy).sqrt();
        if length <= 1e-6 {
            dx = px - centre.0;
            dy = py - centre.1;
            length = (dx * dx + dy * dy).sqrt();
            if length <= 1e-6 {
                (dx, dy, length) = (1.0, 0.0, 1.0);
            }
        }
        let reach = scene.nodes[parent].radius + WEDGE_TETHER_PX + scene.nodes[index].radius;
        let radius = scene.nodes[index].radius;
        let x = (px + dx / length * reach).clamp(radius, (width - radius).max(radius));
        let y = (py + dy / length * reach).clamp(radius, (height - radius).max(radius));
        positions.xy[index] = (x, y);
        // Outward from the parent, so the count never sits on top of the name
        // that makes it mean something.
        if let Some(side) = positions.label_side.get_mut(index) {
            *side = if x < px {
                layout::LabelSide::Left
            } else {
                layout::LabelSide::Right
            };
        }
    }
}

/// Smallest radius the node a layout is centred on may be drawn at.
///
/// An instance node's radius is a constant ([`encoding::INSTANCE_RADIUS_PX`]) —
/// it encodes nothing, because an instance carries no count — so raising it for
/// the ego centre takes no meaning away from the picture. On a meta-graph
/// centre the type's own radius is an encoding and wins if it is already
/// larger, which is what `max` says.
const EGO_RADIUS_PX: f64 = 15.0;

/// Mark the node (or nodes) a radial kernel will centre, so the emitter can
/// draw them as centres.
///
/// **The plan decides this, not the request.** `Scene::seeds` is what the
/// caller *named*; the centre is what `structure::plan` will actually put in
/// the middle, and for an island packing there is one per star-shaped island
/// that no request named at all.
///
/// The island bucketing mirrors `layout::islands` exactly — same singleton
/// rule, same member order — because the two have to agree about which local
/// index is which. Both are pure functions of `(community, links)`, so they
/// agree by construction rather than by a shared mutable structure.
fn emphasise_centres(scene: &mut Scene, plan: &structure::Plan, links: &[(usize, usize)]) {
    let mut centres: Vec<usize> = Vec::new();
    match plan {
        structure::Plan::Radial { seed } => centres.push(*seed),
        structure::Plan::Islands { community, count } => {
            let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); *count];
            for (index, group_id) in community.iter().enumerate() {
                if *group_id < *count {
                    buckets[*group_id].push(index);
                }
            }
            for members in buckets.iter().filter(|b| b.len() > 1) {
                let position_of: std::collections::HashMap<usize, usize> = members
                    .iter()
                    .enumerate()
                    .map(|(local, global)| (*global, local))
                    .collect();
                let local_links: Vec<(usize, usize)> = links
                    .iter()
                    .filter_map(|(a, b)| Some((*position_of.get(a)?, *position_of.get(b)?)))
                    .collect();
                if let structure::Plan::Radial { seed } =
                    structure::plan(members.len(), &local_links, &[])
                {
                    centres.push(members[seed]);
                }
            }
        }
        structure::Plan::Force => {}
    }
    for centre in centres {
        let Some(node) = scene.nodes.get_mut(centre) else {
            continue;
        };
        // A folded fan is never a centre: it stands for nodes that are not in
        // the picture, and a halo would claim the opposite.
        if node.aggregate.is_some() {
            continue;
        }
        node.emphasis = true;
        node.pinned = true;
        node.radius = node.radius.max(EGO_RADIUS_PX);
    }
}

/// The arc key each node is grouped by on a hop ring — its type.
///
/// Interned to a `u32` here rather than compared as strings inside the layout:
/// the layout sorts by this key on every ring, and the key's *value* never
/// reaches the picture, only its equality does. First-appearance order, so it
/// is a function of the scene and not of a hash seed.
fn arc_groups(scene: &Scene) -> Vec<u32> {
    let mut seen: Vec<&str> = Vec::new();
    scene
        .nodes
        .iter()
        .map(|node| {
            let name = node.node_type.as_deref().unwrap_or("");
            match seen.iter().position(|s| *s == name) {
                Some(index) => index as u32,
                None => {
                    seen.push(name);
                    (seen.len() - 1) as u32
                }
            }
        })
        .collect()
}

/// Fan-out threshold: below this many same-type leaves on one parent, the
/// nodes are drawn.
///
/// **The number is a legibility measurement, not a taste.** Twenty-five dots
/// around a parent still resolve as twenty-five things at 800x500; past that
/// they merge into a smear whose only readable property is "lots", and a glyph
/// that says `Wellbore x 61` carries strictly more than the smear does — it
/// carries the exact number.
const FAN_THRESHOLD: usize = 25;

/// Canvas pixels per node, above which the picture has room and nothing is
/// folded at all.
///
/// **Folding a picture that had space for its nodes is a loss, not a
/// summary.** The 150-node bipartite result fits on a 1600x1000 canvas with
/// room to read every wellbore name; folding three of its fields into wedges
/// there threw away 121 nodes to save space nothing needed. The threshold
/// above says *when a fan is too big to read*; this one says *when the image
/// is too full to hold it*, and both have to be true.
const FOLD_ABOVE_PX_PER_NODE: f64 = 6_000.0;

/// Fold every same-type fan of leaves into one wedge (P11 direction (e)).
///
/// A **leaf** here is a node with exactly one link in this scene. That is a
/// statement about the picture, not about the graph: the node may have a
/// thousand edges the bound never fetched, and this does not claim otherwise —
/// which is why the glyph says how many nodes it stands for and how many of
/// them are drawn, rather than pretending to summarise the graph.
///
/// **Render-side only.** The protocol, the live view and every count in the
/// response are untouched; this runs on a `Scene` that already exists and is
/// about to be drawn. An aggregation that reached the slot space would change
/// what a later `expand` means.
///
/// Returns how many nodes were folded away.
fn fold_leaf_fans(scene: &mut Scene, canvas_area: f64) -> u32 {
    let count = scene.nodes.len();
    if (count as f64) * FOLD_ABOVE_PX_PER_NODE <= canvas_area {
        return 0;
    }
    let links: Vec<(usize, usize)> = scene.links.iter().map(|l| (l.source, l.target)).collect();
    let degree = structure::degrees(count, &links);

    // parent -> its degree-1 children, in index order.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (a, b) in &links {
        if *a >= count || *b >= count || a == b {
            continue;
        }
        if degree[*a] == 1 && degree[*b] > 1 {
            children[*b].push(*a);
        } else if degree[*b] == 1 && degree[*a] > 1 {
            children[*a].push(*b);
        }
    }

    let mut folded_into: Vec<Option<usize>> = vec![None; count];
    let mut fans: Vec<(usize, String, Vec<usize>)> = Vec::new();
    for (parent, kids) in children.iter().enumerate() {
        // Grouped by type, in first-appearance order, so the grouping does not
        // depend on a map's iteration.
        let mut by_type: Vec<(String, Vec<usize>)> = Vec::new();
        for kid in kids {
            // A node the layout is about to centre is never folded, whatever
            // its degree: the request asked for it by name.
            if scene.seeds.contains(kid) || scene.nodes[*kid].aggregate.is_some() {
                continue;
            }
            let name = scene.nodes[*kid].node_type.clone().unwrap_or_default();
            match by_type.iter_mut().find(|(t, _)| *t == name) {
                Some((_, group)) => group.push(*kid),
                None => by_type.push((name, vec![*kid])),
            }
        }
        for (node_type, group) in by_type {
            if group.len() <= FAN_THRESHOLD {
                continue;
            }
            fans.push((parent, node_type, group));
        }
    }
    if fans.is_empty() {
        return 0;
    }

    let mut folded = 0u32;
    for (fan, (parent, node_type, group)) in fans.iter().enumerate() {
        // A wedge hanging off an unnamed dot says "twenty-seven of something
        // are attached to *that*" without saying what `that` is. The parent's
        // label is part of the aggregate's honesty, so it is pinned with it.
        scene.nodes[*parent].pinned = true;
        let glyph = scene.nodes.len();
        let total = group.len() as u32;
        folded += total;
        for kid in group {
            folded_into[*kid] = Some(glyph);
        }
        scene.nodes.push(SceneNode {
            // Its own slot space: a real slot would be a lie an agent could
            // pass back to `expand`. Numbered above every real slot, so the
            // label grid's tie-break stays total.
            slot: u32::MAX - fan as u32,
            text: format!(
                "{} × {} (showing {})",
                if node_type.is_empty() {
                    "nodes"
                } else {
                    node_type
                },
                encoding::group_thousands(u64::from(total)),
                "none"
            ),
            weight: u64::from(total),
            radius: encoding::aggregate_radius(total),
            color: encoding::AGGREGATE_COLOR,
            badges: Vec::new(),
            dimmed: false,
            node_type: Some(node_type.clone()),
            // The count is inside the text, where the words "showing none"
            // qualify it. A second bare number in a chip beside it would read
            // as a second, different fact.
            show_count: false,
            pinned: true,
            aggregate: Some(total),
            emphasis: false,
        });
    }

    // Rebuild: drop the folded nodes, remap every surviving index, and rewrite
    // each folded child's link to point at its glyph.
    let mut new_index: Vec<Option<usize>> = vec![None; scene.nodes.len()];
    let mut kept: Vec<SceneNode> = Vec::with_capacity(scene.nodes.len());
    for (index, node) in scene.nodes.iter().enumerate() {
        if index < count && folded_into[index].is_some() {
            continue;
        }
        new_index[index] = Some(kept.len());
        kept.push(node.clone());
    }
    let resolve = |index: usize| -> Option<usize> {
        let target = folded_into.get(index).copied().flatten().unwrap_or(index);
        new_index[target]
    };
    let mut links_out: Vec<SceneLink> = Vec::with_capacity(scene.links.len());
    for link in &scene.links {
        let (Some(source), Some(target)) = (resolve(link.source), resolve(link.target)) else {
            continue;
        };
        if source == target {
            continue;
        }
        // One line per parent-to-glyph pair, not one per folded child: the
        // count is on the glyph, and forty coincident lines are forty times the
        // ink for no information.
        if links_out
            .iter()
            .any(|l| l.source == source && l.target == target)
        {
            continue;
        }
        links_out.push(SceneLink {
            source,
            target,
            width: link.width,
        });
    }
    scene.nodes = kept;
    scene.links = links_out;
    scene.seeds = scene
        .seeds
        .iter()
        .filter_map(|s| new_index.get(*s).copied().flatten())
        .collect();
    folded
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
            Ok(slice_scene(session, &slice, None))
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
            Ok(slice_scene(session, &slice, Some(&expand.node_type)))
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
                    node_type: Some(name.clone()),
                    show_count: true,
                    pinned: false,
                    aggregate: None,
                    emphasis: false,
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
                    color: encoding::type_hue(node_type),
                    badges: Vec::new(),
                    dimmed: false,
                    node_type: Some(node_type.clone()),
                    // An instance node's count is always 1, and a chip that
                    // says so every time is width spent on nothing.
                    show_count: false,
                    pinned: false,
                    aggregate: None,
                    emphasis: false,
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
        // A live view is whatever the user navigated to; the request that drew
        // it named no origin, so the layout is chosen from the shape alone.
        seeds: Vec::new(),
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
            node_type: Some(n.name.clone()),
            // A type node's count is the whole reason it is that size.
            show_count: true,
            pinned: false,
            aggregate: None,
            emphasis: false,
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
        seeds: Vec::new(),
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
fn slice_scene(session: &Session, slice: &GraphSlice, seed_type: Option<&str>) -> Scene {
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
            color: encoding::type_hue(&n.node_type),
            badges: Vec::new(),
            dimmed: false,
            node_type: Some(n.node_type.clone()),
            show_count: false,
            pinned: false,
            aggregate: None,
            emphasis: false,
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

    let seeds: Vec<usize> = match seed_type {
        Some(name) => slice
            .meta
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.node_type == name)
            .map(|(i, _)| i)
            .collect(),
        None => Vec::new(),
    };

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
        // What the *request* called its origin. An expansion out of one type
        // seeds every node of that type, so this is usually a long list and the
        // layout treats it as "no centre named" — see `Scene::seeds`. It is one
        // entry exactly when the type has one member, which is the ego case.
        seeds,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node(text: &str, radius: f64, aggregate: Option<u32>) -> SceneNode {
        SceneNode {
            slot: 0,
            text: text.to_string(),
            weight: 1,
            radius,
            color: encoding::INSTANCE_COLOR,
            badges: Vec::new(),
            dimmed: false,
            node_type: Some("T".to_string()),
            show_count: false,
            pinned: false,
            aggregate,
            emphasis: false,
        }
    }

    /// A folded fan sits beside the node it hangs off, whatever the layout did.
    ///
    /// The failure this fixes has a picture: a wedge reading `Wellbore x 27`
    /// four hundred pixels from any parent, with its one connector lost among
    /// the other lines, so the picture said twenty-seven of something attach to
    /// *nothing in particular*.
    #[test]
    fn a_folded_fan_is_moored_to_its_parent() {
        let scene = Scene {
            nodes: vec![node("parent", 10.0, None), node("T x 27", 20.0, Some(27))],
            links: vec![SceneLink {
                source: 0,
                target: 1,
                width: 1.0,
            }],
            status: Vec::new(),
            banners: Vec::new(),
            place_all_labels: false,
            seeds: Vec::new(),
        };
        let mut positions = layout::Positions {
            xy: vec![(200.0, 300.0), (900.0, 300.0)],
            label_side: vec![layout::LabelSide::Below; 2],
            islands: Vec::new(),
        };
        moor_folded_fans(&scene, &mut positions, 1000.0, 600.0);
        assert_eq!(positions.xy[0], (200.0, 300.0), "the parent does not move");
        let (x, y) = positions.xy[1];
        let gap = ((x - 200.0f64).powi(2) + (y - 300.0f64).powi(2)).sqrt();
        assert!(
            (gap - (10.0 + WEDGE_TETHER_PX + 20.0)).abs() < 1e-6,
            "the wedge sits one tether from its parent, not {gap}"
        );
        assert!(y == 300.0 && x > 200.0, "and along the direction it was on");
        assert_eq!(
            positions.label_side[1],
            layout::LabelSide::Right,
            "its count is drawn away from the parent's own name"
        );
    }

    /// A glyph the layout dropped on top of its parent still lands somewhere,
    /// and somewhere is away from the crowd rather than at an arbitrary angle.
    #[test]
    fn a_coincident_fan_is_pushed_outward_rather_than_left_on_its_parent() {
        let scene = Scene {
            nodes: vec![
                node("far", 6.0, None),
                node("parent", 10.0, None),
                node("T x 9", 15.0, Some(9)),
            ],
            links: vec![SceneLink {
                source: 1,
                target: 2,
                width: 1.0,
            }],
            status: Vec::new(),
            banners: Vec::new(),
            place_all_labels: false,
            seeds: Vec::new(),
        };
        let mut positions = layout::Positions {
            // Centroid sits left of the parent, so "away from the centre" is
            // rightward and the assertion below is not the default direction.
            xy: vec![(100.0, 300.0), (700.0, 300.0), (700.0, 300.0)],
            label_side: vec![layout::LabelSide::Below; 3],
            islands: Vec::new(),
        };
        moor_folded_fans(&scene, &mut positions, 1000.0, 600.0);
        assert!(
            positions.xy[2].0 > 700.0,
            "a coincident glyph goes outward: {:?}",
            positions.xy[2]
        );
    }
}
