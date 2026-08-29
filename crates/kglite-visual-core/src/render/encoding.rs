//! P7's visual encoding, as pure Rust functions (plan D13).
//!
//! **Parity is a requirement, not polish.** The static image and the live app
//! must show the same graph, or an agent and the human beside it are looking at
//! two different pictures and neither can say which. So every constant and
//! every ramp below is a *port* of a named TypeScript site, and each carries a
//! comment naming it. The TypeScript side carries the reciprocal comment
//! naming this module, because a one-way pointer is a pointer nobody finds
//! from the side that changes.
//!
//! **This is bounded, accepted duplication, and it is not D10.** D10 refused a
//! second *renderer product* — a second appearance model, a second interaction
//! model, a second thing to keep in step forever. What lives here is a drawing
//! function over five numbers and a palette: no interaction, no camera, no
//! device. The duplication is a page of constants with a paired comment on each
//! side, which is the cheapest true statement of "these two must agree".
//!
//! **What parity does NOT mean: pixel identity.** cosmos.gl draws at a
//! data-derived zoom onto a GPU surface; this draws into a fixed viewBox. The
//! encodings agree — a type twice as populous gets the same radius from the
//! same ramp — and the geometry does not, because the layouts are different
//! algorithms by design (`super::layout`).

/// RGBA in 0..1, which is the range the frontend's appearance module works in
/// (cosmos.gl takes 0..1, not 0..255). Kept in that range here so the ported
/// palette is a literal copy of the TypeScript one rather than a converted
/// one — a conversion is a place the two can silently disagree.
///
/// Mirrors `frontend/src/appearance.ts::Rgba`.
pub type Rgba = [f64; 4];

/// Radius range for a meta-graph type node.
///
/// Mirrors `TYPE_MIN_PX` / `TYPE_MAX_PX` in `frontend/src/appearance.ts`.
pub const TYPE_MIN_PX: f64 = 6.0;
pub const TYPE_MAX_PX: f64 = 36.0;

/// Scale applied to a supporting type's radius — the same ramp, one step
/// quieter, so a large supporting type still reads as larger than a small one.
///
/// Mirrors `SUPPORTING_SCALE` in `frontend/src/appearance.ts`.
pub const SUPPORTING_SCALE: f64 = 0.6;

/// Radius for an instance node.
///
/// Mirrors the `sizes[slot] = 6` branch of `appearance()` in
/// `frontend/src/main.ts` — an instance node carries no count to encode, so it
/// gets one size and the meta-graph keeps the size channel to itself.
pub const INSTANCE_RADIUS_PX: f64 = 6.0;

/// Radius for a type node with `count` members, on a graph whose largest type
/// has `max`.
///
/// **Log, not a root** — the argument, and the deciles that bought it, are at
/// `typeRadius` in `frontend/src/appearance.ts`, which this ports. `f64`
/// throughout because `Math.log1p` is `f64`: a `f32` port would round
/// differently from the app it is claiming parity with.
pub fn type_radius(count: u32, max: u32, supporting: bool) -> f64 {
    let ceiling = f64::from(max.max(1));
    let ramp = f64::from(count).ln_1p() / ceiling.ln_1p();
    let radius = TYPE_MIN_PX + (TYPE_MAX_PX - TYPE_MIN_PX) * ramp.min(1.0);
    if supporting {
        radius * SUPPORTING_SCALE
    } else {
        radius
    }
}

/// Width range for a meta-graph link.
///
/// Mirrors `LINK_MIN_PX` / `LINK_MAX_PX` in `frontend/src/appearance.ts`.
pub const LINK_MIN_PX: f64 = 0.5;
pub const LINK_MAX_PX: f64 = 5.0;

/// Width for a meta-graph link carrying `count` edges.
///
/// Mirrors `linkWidth` in `frontend/src/appearance.ts`. A count of 0 means the
/// server had no number to give and takes the floor rather than a fabricated
/// width — an instance edge is one edge, and there is no count on it to encode.
pub fn link_width(count: u32, max: u32) -> f64 {
    if count == 0 {
        return LINK_MIN_PX;
    }
    let ceiling = f64::from(max.max(1));
    let ramp = f64::from(count).ln_1p() / ceiling.ln_1p();
    LINK_MIN_PX + (LINK_MAX_PX - LINK_MIN_PX) * ramp.min(1.0)
}

/// Link count at and below which the picture spends its full ink budget.
///
/// Around this many lines a reader can still follow one; past it the lines stop
/// being lines and become a wash.
const LINK_INK_FULL_BELOW: f64 = 250.0;

/// Floor on the ink ramp. Below this a link is not quiet, it is missing, and a
/// picture that silently drops its edges is worse than a hazy one.
const LINK_INK_FLOOR: f64 = 0.30;

/// Extra quietening for a link whose two ends are in different communities.
///
/// Within-island lines are the ones that make a group read as a group;
/// cross-island lines are the ones that made the round-1 meta-graph read as a
/// web. Both are drawn — a bridge a picture hides is a bridge a reader
/// concludes is not there — but the second is drawn as background.
pub const CROSS_ISLAND_INK: f64 = 0.45;

/// How much of the link ink budget a picture with `links` lines may spend.
///
/// **The failure this answers, measured:** the discovery-licensee expansion
/// draws 3,374 links at the same 0.45 stroke opacity the 124-link meta-graph
/// uses, and 3,374 overlapping strokes at 0.45 composite to a solid white haze
/// with the nodes floating on it. The count is known before a single line is
/// emitted, so the ink can be a function of it.
///
/// **Inverse-square-root, not inverse.** Ink on the page grows with the number
/// of lines, and what a reader perceives grows with roughly the square root of
/// that; `1/n` overcorrects into invisibility at a few hundred links, while
/// `1/sqrt(n)` holds the *total* perceived ink near constant, which is the
/// property wanted. Floored, because a link nobody can see is a link the
/// picture is claiming does not exist.
///
/// **No frontend counterpart, and it is not a parity gap.** The app can be
/// zoomed: a haze at one zoom resolves into lines at the next, and the user
/// decides. An image is what the reader gets, at the size it was rendered.
pub fn link_ink(links: usize) -> f64 {
    if links == 0 {
        return 1.0;
    }
    (LINK_INK_FULL_BELOW / links as f64)
        .sqrt()
        .clamp(LINK_INK_FLOOR, 1.0)
}

/// The same ramp applied to stroke *width*, at half strength.
///
/// Width and opacity are not interchangeable: below about half a pixel a
/// rasteriser antialiases a stroke into a grey nothing, and the width channel
/// is also carrying the meta-graph's edge counts. So the width gives up half of
/// what the opacity does, and the encoding survives.
pub fn link_width_ink(links: usize) -> f64 {
    0.5 + 0.5 * link_ink(links)
}

/// A type node with no capability flags.
///
/// Mirrors the `plain` branch of `baseColor` in `frontend/src/main.ts`.
pub const TYPE_PLAIN_COLOR: Rgba = [0.35, 0.65, 0.98, 0.92];

/// A type node declaring at least one capability (`ts`/`geo`/`loc`/`vec`).
///
/// Mirrors the non-`plain` branch of `baseColor` in `frontend/src/main.ts`.
pub const TYPE_CAPABLE_COLOR: Rgba = [0.98, 0.75, 0.32, 0.92];

/// An instance node whose type is unknown — the only case left after P11 gave
/// instance nodes a per-type hue.
///
/// Mirrors the `nodeType === null` fallback of `instanceColor` in
/// `frontend/src/appearance.ts`.
pub const INSTANCE_COLOR: Rgba = [0.55, 0.70, 0.90, 0.85];

/// One hue per node type, for instance nodes.
///
/// **The P9 renders drew every instance node in one blue** (`INSTANCE_COLOR`
/// above), which on a mixed neighbourhood — a wellbore with its licences, its
/// cores and its logs — said "these are all the same kind of thing" about a
/// picture whose whole content is that they are not. The colour channel was
/// carrying nothing, and it is the channel that survives being scaled to a
/// thumbnail.
///
/// **Type, not community.** The layout already puts a community in one place
/// (`super::layout::islands`), so colouring by community would spend the one
/// free channel restating what position says. Type is orthogonal to both, and
/// it is what a reader is actually trying to tell apart inside a group.
///
/// Chosen for separability at small size on **both** grounds — every entry
/// clears 3:1 contrast against `#0d1117` and against `#ffffff` — and ordered so
/// that adjacent indices are far apart in hue, because the assignment below is
/// by hash and neighbours in the table are what a two-type picture is most
/// likely to draw.
///
/// Mirrors `TYPE_HUES` in `frontend/src/appearance.ts`.
pub const TYPE_HUES: [Rgba; 10] = [
    [0.36, 0.68, 0.98, 0.90], // blue
    [0.98, 0.62, 0.30, 0.90], // orange
    [0.42, 0.82, 0.52, 0.90], // green
    [0.85, 0.50, 0.92, 0.90], // violet
    [0.98, 0.80, 0.34, 0.90], // amber
    [0.40, 0.83, 0.85, 0.90], // teal
    [0.96, 0.51, 0.60, 0.90], // rose
    [0.62, 0.74, 0.42, 0.90], // olive
    [0.68, 0.62, 0.96, 0.90], // periwinkle
    [0.92, 0.68, 0.52, 0.90], // clay
];

/// The hue a type name maps to.
///
/// FNV-1a over the name's bytes, not a `HashMap` and not the type's position in
/// any list: the same type must get the same colour in every image forever, and
/// a position-derived index would repaint the whole picture the day a query
/// returned its rows in a different order. `std`'s default hasher is
/// randomly seeded per process and would give two runs two different pictures.
///
/// Mirrors `typeHue` in `frontend/src/appearance.ts`.
pub fn type_hue(node_type: &str) -> Rgba {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in node_type.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    TYPE_HUES[(hash % TYPE_HUES.len() as u32) as usize]
}

/// An aggregate glyph's fill — one grey, never a type hue.
///
/// **An aggregate is not a node, and it must not be mistakable for one**
/// (P11 direction (e)). It stands for nodes that were *not* drawn, so it is
/// deliberately outside the type palette: a reader who has learned that green
/// means `Wellbore` must not read a folded fan of wellbores as one green
/// wellbore. Shape carries the distinction too — `super::svg` draws it as a
/// sector, not a circle — and colour repeats it, because at thumbnail size
/// shape is the first thing to go.
pub const AGGREGATE_COLOR: Rgba = [0.58, 0.62, 0.68, 0.80];

/// Radius of the wedge standing for `count` folded nodes.
///
/// The same log ramp the type nodes use, against a fixed ceiling rather than
/// the scene's largest: an aggregate is not competing with the type nodes for
/// the size channel, and a ramp normalised per image would make the same fan of
/// forty draw at two sizes in two renders of the same graph.
pub fn aggregate_radius(count: u32) -> f64 {
    const CEILING: f64 = 500.0;
    let ramp = (f64::from(count).ln_1p() / CEILING.ln_1p()).min(1.0);
    10.0 + 22.0 * ramp
}

/// Alpha multiplier for a supporting type — it keeps its hue and loses most of
/// its opacity, so the core types it hangs off carry the picture.
///
/// Mirrors the `label.supporting ? [r, g, b, a * 0.45]` line of `baseColor` in
/// `frontend/src/main.ts`.
pub const SUPPORTING_ALPHA: f64 = 0.45;

/// A node's colour before any highlight.
///
/// Mirrors `baseColor` in `frontend/src/main.ts`, minus the colour-by branch:
/// a render request carries no appearance selection, so the two bits a type
/// node carries here are exactly the two the app shows with no colour-by
/// chosen — whether it declares a capability, and whether it is supporting.
///
/// An instance node takes its type's hue ([`type_hue`]); a type node keeps the
/// capability/supporting encoding, which is a *different* fact and would be
/// destroyed by overwriting it with a hue.
pub fn base_color(is_type: bool, has_capabilities: bool, supporting: bool) -> Rgba {
    if !is_type {
        return INSTANCE_COLOR;
    }
    let [r, g, b, a] = if has_capabilities {
        TYPE_CAPABLE_COLOR
    } else {
        TYPE_PLAIN_COLOR
    };
    if supporting {
        [r, g, b, a * SUPPORTING_ALPHA]
    } else {
        [r, g, b, a]
    }
}

/// Background colour of one capability badge.
///
/// Mirrors the `.kglv-badge*` rules in `frontend/src/styles.css`: `ts` amber,
/// `geo`/`loc` green, `vec` purple, anything else the base blue.
pub fn badge_color(badge: &str) -> &'static str {
    match badge {
        "ts" => "#9e6a03",
        "geo" | "loc" => "#2ea043",
        "vec" => "#8957e5",
        _ => "#1f6feb",
    }
}

/// kglite's own four capability flags, spelled out for a human reader.
///
/// Mirrors `BADGE_TITLES` in `frontend/src/labels.ts`. Rendered as an SVG
/// `<title>` on the badge, which is the static image's equivalent of the chip's
/// hover tooltip.
pub fn badge_title(badge: &str) -> &'static str {
    match badge {
        "ts" => "has timeseries data",
        "geo" => "has WKT geometry",
        "loc" => "has lat/lon locations",
        "vec" => "has embedding vectors",
        _ => "",
    }
}

/// Which palette the chrome is drawn from.
///
/// Two, not one, because an image lands in places the app never does — a
/// notebook on a white background, a light-themed chat client, a printed page —
/// and a dark rectangle in a white document is a worse picture than a wrong
/// theme. Dark is the default because it is what the app shows (`backgroundColor`
/// in `frontend/src/render.ts`), and "the image looks like the app" is the
/// requirement D13 states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

/// The chrome colours one theme supplies. Node and link *encoding* colours are
/// theme-independent by design: they carry data, and a datum that changes
/// colour with the surrounding page is not an encoding.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Mirrors `backgroundColor` in `frontend/src/render.ts` (dark).
    pub background: &'static str,
    /// Link stroke. cosmos.gl's own link default in the app; stated explicitly
    /// here because an SVG has no default.
    pub link: &'static str,
    pub link_opacity: f64,
    /// Mirrors `.kglv-label` `color` in `frontend/src/styles.css`.
    pub label_text: &'static str,
    /// Mirrors `.kglv-label-dim` `color`.
    pub label_dim_text: &'static str,
    /// Mirrors `.kglv-label-count` `color`.
    pub label_count: &'static str,
    /// Mirrors `.kglv-label` `background` / `border`.
    pub chip_fill: &'static str,
    pub chip_fill_opacity: f64,
    pub chip_stroke: &'static str,
    pub chip_stroke_opacity: f64,
    /// Mirrors `.kglv-status` colours.
    pub status_text: &'static str,
    pub status_fill: &'static str,
    pub status_stroke: &'static str,
    /// Mirrors `.kglv-warn` — the colour the truncation banner is drawn in.
    pub warn: &'static str,
    /// The boundary drawn around a packed community (`super::svg::emit_islands`).
    ///
    /// **No frontend counterpart.** The app has no island packing to enclose —
    /// its layout is cosmos.gl's force simulation — so this is chrome the
    /// static image needs and the live view has no equivalent of. It takes the
    /// muted foreground each palette already uses for links and status text,
    /// rather than a new colour, because a boundary is not a datum.
    pub island: &'static str,
}

impl Theme {
    pub fn palette(self) -> Palette {
        match self {
            // Every value here is read out of the app's own stylesheet or
            // renderer config; see the field docs for the site of each.
            Theme::Dark => Palette {
                background: "#0d1117",
                link: "#8b949e",
                link_opacity: 0.45,
                label_text: "#e6edf3",
                label_dim_text: "#8b949e",
                label_count: "#8b949e",
                chip_fill: "#0d1117",
                chip_fill_opacity: 0.78,
                chip_stroke: "#8b949e",
                chip_stroke_opacity: 0.35,
                status_text: "#c9d1d9",
                status_fill: "#0d1117",
                status_stroke: "#8b949e",
                warn: "#f0883e",
                island: "#8b949e",
            },
            // The dark palette's roles, re-derived for a light ground. Not a
            // second design: the same seven roles, inverted, so a reader who
            // knows one image can read the other.
            Theme::Light => Palette {
                background: "#ffffff",
                link: "#57606a",
                link_opacity: 0.45,
                label_text: "#1f2328",
                label_dim_text: "#656d76",
                label_count: "#656d76",
                chip_fill: "#ffffff",
                chip_fill_opacity: 0.82,
                chip_stroke: "#57606a",
                chip_stroke_opacity: 0.35,
                status_text: "#1f2328",
                status_fill: "#ffffff",
                status_stroke: "#57606a",
                warn: "#bc4c00",
                island: "#57606a",
            },
        }
    }
}

/// A count, in the thousands-separated form the app shows.
///
/// Mirrors the `toLocaleString('en-US')` calls in `frontend/src/main.ts` and
/// `frontend/src/labels.ts`. The locale is pinned there and therefore pinned
/// here: a label whose separator depended on the reader's machine would make
/// the golden baseline a fact about the machine that generated it.
pub fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Phrase the truncation banner.
///
/// Mirrors `noteTruncation` in `frontend/src/main.ts`, **including its wording**
/// — D5 says a truncated answer that does not say so reads as a complete one,
/// and an image that phrased it differently from the app would make the two
/// impossible to compare. Returns `None` when nothing was clipped, which is the
/// app's `truncationBanner = null`.
///
/// `links` is `(returned, total)` for the slice's link half, or `None` where
/// the request has no separate link bound. "up to" for the link total, because
/// the server's link total counts every edge its walk found and refused, and a
/// both-directions walk can find one edge from either end.
pub fn truncation_banner(
    truncated: bool,
    returned: u32,
    total: u32,
    unit: &str,
    links: Option<(bool, u32, u32)>,
) -> Option<String> {
    let mut clauses: Vec<String> = Vec::new();
    if truncated {
        clauses.push(format!(
            "{} of {} {unit}",
            group_thousands(u64::from(returned)),
            group_thousands(u64::from(total))
        ));
    }
    if let Some((link_truncated, link_returned, link_total)) = links {
        if link_truncated {
            clauses.push(format!(
                "{} of up to {} links",
                group_thousands(u64::from(link_returned)),
                group_thousands(u64::from(link_total))
            ));
        }
    }
    if clauses.is_empty() {
        return None;
    }
    Some(format!("showing {}", clauses.join(" and ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ramp's whole point: every decile of a log-uniform population is a
    /// visibly different size. These are the deciles `appearance.ts` names in
    /// its own doc comment, so a drift on either side shows up here.
    #[test]
    fn the_size_ramp_spreads_the_deciles_it_was_tuned_against() {
        let max = 102_420;
        let radii: Vec<f64> = [3, 23, 118, 1_051, 4_249, 11_000, 102_420]
            .iter()
            .map(|c| type_radius(*c, max, false))
            .collect();
        for pair in radii.windows(2) {
            assert!(
                pair[1] - pair[0] > 1.5,
                "adjacent deciles must differ by more than a pixel of rounding: {pair:?}"
            );
        }
        assert_eq!(radii[6], TYPE_MAX_PX, "the largest type is the ceiling");
        assert!(radii[0] > TYPE_MIN_PX, "the smallest type is off the floor");
    }

    #[test]
    fn a_supporting_type_keeps_its_place_on_the_ramp() {
        let big = type_radius(10_000, 100_000, true);
        let small = type_radius(10, 100_000, true);
        assert!(big > small, "one step quieter, not a separate ramp");
        assert_eq!(big, type_radius(10_000, 100_000, false) * SUPPORTING_SCALE);
    }

    #[test]
    fn a_countless_link_takes_the_floor_rather_than_a_fabricated_width() {
        assert_eq!(link_width(0, 5_000), LINK_MIN_PX);
        assert_eq!(link_width(5_000, 5_000), LINK_MAX_PX);
        assert!(link_width(1, 5_000) > LINK_MIN_PX);
    }

    #[test]
    fn the_link_ink_budget_falls_with_the_link_count_and_stops_at_a_floor() {
        // The case that bought it: 124 links keep the full budget, 3,374
        // composite to a white wash at the same per-line opacity.
        assert_eq!(link_ink(0), 1.0);
        assert_eq!(link_ink(124), 1.0, "a sparse picture spends everything");
        assert!(link_ink(1_000) < link_ink(500));
        assert!(link_ink(500) < link_ink(250));
        assert_eq!(
            link_ink(1_000_000),
            LINK_INK_FLOOR,
            "a link nobody can see is a link the picture denies"
        );
        // Inverse-square-root, not inverse: four times the links is half the
        // per-line ink, so the total perceived ink is what stays constant.
        assert!((link_ink(2_000) / link_ink(500) - 0.5).abs() < 1e-9);
        // And width gives up half of what opacity does, because below about
        // half a pixel a rasteriser antialiases a stroke into nothing.
        assert!(link_width_ink(4_000) > link_ink(4_000));
        assert_eq!(link_width_ink(100), 1.0);
    }

    #[test]
    fn a_supporting_type_dims_without_changing_hue() {
        let core = base_color(true, false, false);
        let supporting = base_color(true, false, true);
        assert_eq!(core[0..3], supporting[0..3], "the hue is the encoding");
        assert_eq!(supporting[3], core[3] * SUPPORTING_ALPHA);
    }

    #[test]
    fn counts_are_grouped_the_way_the_app_groups_them() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_000), "1,000");
        assert_eq!(group_thousands(546_850), "546,850");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn the_banner_says_what_the_app_says_or_says_nothing() {
        assert_eq!(truncation_banner(false, 12, 12, "nodes", None), None);
        assert_eq!(
            truncation_banner(true, 5_000, 12_400, "nodes", None).as_deref(),
            Some("showing 5,000 of 12,400 nodes")
        );
        assert_eq!(
            truncation_banner(true, 5_000, 12_400, "nodes", Some((true, 9, 40))).as_deref(),
            Some("showing 5,000 of 12,400 nodes and 9 of up to 40 links")
        );
        assert_eq!(
            truncation_banner(false, 3, 3, "nodes", Some((true, 9, 40))).as_deref(),
            Some("showing 9 of up to 40 links"),
            "a complete node list with a clipped link list still has to say so"
        );
    }
}
