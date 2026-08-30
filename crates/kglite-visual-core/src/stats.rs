//! Per-type property statistics, and one node's stored properties (plan D12).
//!
//! **Sampled statistics are marked, and the mark survives to the screen.**
//! kglite scans a type exhaustively up to
//! `EXACT_PROPERTY_STATS_MAX_NODES` and samples above it, setting `approx` on
//! every stat it could not enumerate. Presenting a sampled distinct-count as an
//! exact one is the failure Phase 0 named: a color-by menu offering "3 distinct
//! values" for a property with 40 000 is not a smaller truth, it is a wrong
//! one. The flag rides all the way to the UI text.

use kglite::api::introspection::{compute_property_stats, EXACT_PROPERTY_STATS_MAX_NODES};
use kglite::api::{DirGraph, NodeIndex, CANONICAL_NODE_COLUMNS};
use serde::Serialize;
use ts_rs::TS;

use crate::error::CoreError;
use crate::protocol::PROTOCOL_VERSION;
use crate::values::{value_to_display, value_to_json};

/// Distinct values kglite enumerates before it gives up and reports a lower
/// bound. Also the cardinality ceiling for calling a property *categorical*:
/// a color-by menu with more entries than this is a legend nobody reads.
pub const MAX_DISTINCT_VALUES: usize = 32;

/// Nodes sampled when a type is above the exact-scan ceiling.
///
/// kglite's own sampler takes every `total/n`-th node, so this is a stride, not
/// a random draw — deterministic, which is what a committed baseline needs, and
/// blind to any ordering correlation in the type index, which is what it costs.
pub const SAMPLE_SIZE: usize = 20_000;

/// What an appearance channel can be driven by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum AppearanceRole {
    /// Numeric and non-constant: usable for size-by and for a continuous ramp.
    Numeric,
    /// Few enough distinct values to give each one a colour.
    Categorical,
    /// Neither. Listed anyway, with the reason — a property missing from the
    /// menu with no explanation reads as a bug in the menu.
    Unsuitable,
}

/// One property's statistics.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct PropertyStat {
    pub name: String,
    /// kglite's own type string for the column.
    pub value_type: String,
    /// Nodes where this property is set.
    pub non_null: u32,
    /// Distinct values. A **lower bound** when `approx` is true.
    pub unique: u32,
    /// The distinct values, when there were few enough to enumerate.
    #[ts(type = "unknown[]")]
    pub values: Vec<serde_json::Value>,
    /// One example, when there were not.
    #[ts(type = "unknown | null")]
    pub sample: Option<serde_json::Value>,
    /// True when `unique` and `values` are not exhaustive — the population was
    /// sampled, or the distinct-value set hit its cap. The UI must say
    /// "approximate"; presenting this as exact is the failure this field exists
    /// to prevent.
    pub approx: bool,
    pub role: AppearanceRole,
}

/// Per-type property statistics for the appearance menus.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct PropertyStatsResponse {
    pub protocol_version: u32,
    pub node_type: String,
    pub node_count: u32,
    /// True when kglite sampled rather than scanned — i.e. `node_count` is
    /// above its exact ceiling. Carried at the response level as well as per
    /// stat so a UI can label the whole panel once.
    pub sampled: bool,
    /// The ceiling that decision was made against, so the label can say why.
    pub exact_scan_ceiling: u32,
    pub properties: Vec<PropertyStat>,
    /// Properties suitable for size-by, largest `non_null` first.
    pub numeric_candidates: Vec<String>,
    /// Properties suitable for color-by.
    pub categorical_candidates: Vec<String>,
    /// The property this type's nodes would be best *named* by, when one is
    /// clearly better than kglite's `title` (plan E11).
    ///
    /// **The failure it answers is a screen of identical labels.** kglite picks
    /// a title column per type, and on a real schema that choice is sometimes a
    /// code: sodir draws forty wellbores as forty numbers, and the name a
    /// geologist would recognise is sitting in a neighbouring column nobody
    /// looked at. The client applies this to its label overlay — see
    /// [`caption_candidate`] for how the choice is scored, and note it is a
    /// *suggestion*: the panel offers an override, because a heuristic about
    /// what a human finds readable is not a fact the server owns.
    ///
    /// `None` when nothing beat the title, which is the common case and the
    /// right default.
    pub caption_candidate: Option<String>,
}

/// Compute the statistics for one node type.
pub fn property_stats(
    graph: &DirGraph,
    node_type: &str,
) -> Result<PropertyStatsResponse, CoreError> {
    let node_count = graph
        .type_indices
        .get(node_type)
        .map(|nodes| nodes.len())
        .ok_or_else(|| {
            CoreError::Request(format!("no node type named '{node_type}' in this graph"))
        })?;

    let sampled = node_count > EXACT_PROPERTY_STATS_MAX_NODES;
    let sample_size = sampled.then_some(SAMPLE_SIZE);
    let raw = compute_property_stats(graph, node_type, MAX_DISTINCT_VALUES, sample_size)
        .map_err(CoreError::Request)?;

    let mut properties: Vec<PropertyStat> = raw
        .into_iter()
        .map(|stat| {
            let role = classify(
                &stat.property_name,
                &stat.type_string,
                stat.unique,
                stat.non_null,
                stat.approx,
            );
            PropertyStat {
                name: stat.property_name,
                value_type: stat.type_string,
                non_null: stat.non_null as u32,
                unique: stat.unique as u32,
                values: stat
                    .values
                    .unwrap_or_default()
                    .iter()
                    .map(value_to_json)
                    .collect(),
                sample: stat.sample.as_ref().map(value_to_json),
                approx: stat.approx,
                role,
            }
        })
        .collect();

    // Most-populated first; name breaks ties. The menus are drawn from this
    // order and an e2e assertion reads it, so an order derived from a hash map
    // would make both flap.
    properties.sort_by(|a, b| {
        b.non_null
            .cmp(&a.non_null)
            .then_with(|| a.name.cmp(&b.name))
    });

    let numeric_candidates = properties
        .iter()
        .filter(|p| p.role == AppearanceRole::Numeric)
        .map(|p| p.name.clone())
        .collect();
    let categorical_candidates = properties
        .iter()
        .filter(|p| p.role == AppearanceRole::Categorical)
        .map(|p| p.name.clone())
        .collect();

    let caption_candidate = caption_candidate(&properties, node_count as u32);

    Ok(PropertyStatsResponse {
        protocol_version: PROTOCOL_VERSION,
        node_type: node_type.to_string(),
        node_count: node_count as u32,
        sampled,
        exact_scan_ceiling: EXACT_PROPERTY_STATS_MAX_NODES as u32,
        properties,
        numeric_candidates,
        categorical_candidates,
        caption_candidate,
    })
}

/// Coverage a property needs before it can be a caption.
///
/// A name that two thirds of the nodes lack is not a name for the type; it is a
/// name for a subset, and the labels it produced would be a screen where most
/// chips fell back to the title and a few did not — which reads as a rendering
/// bug rather than as a data fact.
const MIN_CAPTION_COVERAGE: f64 = 0.9;

/// Longest sample value a caption may have, in characters.
///
/// A label chip is 130 px wide (`render::labels::CELL_WIDTH`), which is about
/// eighteen characters before the estimate starts claiming its neighbours'
/// cells. Twice that is generous — a name may be long — and a description is
/// still excluded.
const MAX_CAPTION_SAMPLE_CHARS: usize = 40;

/// Property-name fragments that mean "this is what a human calls it".
///
/// Norwegian included, and not as a courtesy: sodir is this project's real
/// graph and its columns are `wlbWellboreName`, `fldName`, `cmpLongName`. A
/// heuristic tuned only on English would score the whole corpus at zero and
/// then be described as "not finding anything", which is a wrong conclusion
/// about the data rather than about the rule. Matched case-insensitively as
/// substrings, so `wlbWellboreName` and `long_name` both hit.
const CAPTION_NAME_HINTS: [&str; 6] = ["name", "navn", "title", "label", "tittel", "caption"];

/// Choose the property a type's nodes read best under, or `None`.
///
/// Pure, and separate from the kglite call for the reason [`classify`] is: the
/// ranking is a judgement about human legibility and it needs a test that pins
/// each rule against a synthetic stat rather than against whatever a
/// half-million-node graph happens to contain.
///
/// **The first rule is that the title has to be inadequate**, and it was added
/// after driving sodir rather than reasoned into place. Without it the ranking
/// answers "which column looks most like a name", which on a real schema is a
/// question with a confident wrong answer: sodir's `Field` nodes are titled
/// `EKOFISK` and carry a `wlbName` of `25/1-4`; its `Wellbore` nodes are titled
/// `6407/1-3` and carry a `wlbLicenceTargetName` of `050`. Both were offered,
/// and both would have replaced a good name with a code on every chip on the
/// screen. A caption exists to rescue a type whose title is *not* a name; where
/// the title already is one, the honest answer is `None`.
///
/// Inadequate means it fails one of the two tests every candidate must pass, so
/// "better than the title" is measured on the axes the title failed rather than
/// on a second opinion about which column sounds nicer. On sodir that is 17 of
/// 98 types: `WellboreCasing` titles 8 620 nodes with 8 distinct values, and
/// `wlbName` is what a reader needs.
///
/// The rules, in the order they eliminate:
///
/// 1. **The title must be inadequate**, by rules 4 and 5 below.
/// 2. **Identity columns are never captions.** `id`, `type` and their siblings
///    are the columns a title would already have been drawn from.
/// 3. **Strings only.** A number is not a name, however well populated.
/// 4. **Covered.** [`MIN_CAPTION_COVERAGE`] of the type's nodes must carry it.
/// 5. **Distinguishing.** A property with a handful of distinct values is a
///    *category* — it is what colour-by is for — and forty nodes all captioned
///    "EXPLORATION" is worse than forty codes, because at least the codes
///    differ. So the caption must look near-unique, which for a capped
///    distinct-count means "it hit the cap": `approx` with `unique` at
///    [`MAX_DISTINCT_VALUES`] is precisely kglite saying "more than I counted".
/// 6. **Short enough to draw.** A sample longer than
///    [`MAX_CAPTION_SAMPLE_CHARS`] is a description.
///
/// What survives is ranked by a name hint first ([`CAPTION_NAME_HINTS`]), then
/// by coverage, then by name so two identical candidates never swap between
/// two calls.
pub(crate) fn caption_candidate(properties: &[PropertyStat], node_count: u32) -> Option<String> {
    if node_count == 0 {
        return None;
    }
    // Rule 1. A type whose title is already a well-covered, distinguishing name
    // needs no caption, and offering one would replace a good name with a code.
    if properties
        .iter()
        .any(|stat| stat.name == "title" && covers(stat, node_count) && distinguishes(stat))
    {
        return None;
    }
    // `Reverse` on the name so a tie on the two real criteria goes to the
    // alphabetically FIRST property under a `max` comparison: the answer must
    // not depend on which order the stats happened to be sorted in.
    let mut best: Option<(bool, u32, std::cmp::Reverse<&str>)> = None;
    for stat in properties {
        if CANONICAL_NODE_COLUMNS.contains(&stat.name.as_str()) {
            continue;
        }
        if !matches!(stat.value_type.as_str(), "String" | "string" | "Utf8") {
            continue;
        }
        if !covers(stat, node_count) || !distinguishes(stat) {
            continue;
        }
        if sample_chars(stat).is_some_and(|len| len > MAX_CAPTION_SAMPLE_CHARS) {
            continue;
        }
        let candidate = (
            has_name_hint(&stat.name),
            stat.non_null,
            std::cmp::Reverse(stat.name.as_str()),
        );
        if best.is_none_or(|current| candidate > current) {
            best = Some(candidate);
        }
    }
    // An unhinted candidate is a guess about a column nobody named "name".
    // Guessing wrong replaces a title the graph's author chose with one this
    // heuristic liked, so the bar for overriding them is the hint.
    best.filter(|(hinted, _, _)| *hinted)
        .map(|(_, _, name)| name.0.to_string())
}

/// Does this property reach [`MIN_CAPTION_COVERAGE`] of the type's nodes?
fn covers(stat: &PropertyStat, node_count: u32) -> bool {
    f64::from(stat.non_null) >= MIN_CAPTION_COVERAGE * f64::from(node_count)
}

/// Is it near-unique rather than a category?
///
/// A capped distinct-count is the interesting half: kglite stops enumerating at
/// [`MAX_DISTINCT_VALUES`] and sets `approx`, so a genuinely unique name column
/// on any type larger than the cap reports exactly the cap. Reading that as
/// "only 32 values, therefore a category" would reject every caption on every
/// type worth captioning.
fn distinguishes(stat: &PropertyStat) -> bool {
    if stat.approx {
        stat.unique as usize >= MAX_DISTINCT_VALUES
    } else {
        u64::from(stat.unique) * 2 >= u64::from(stat.non_null)
    }
}

fn has_name_hint(property: &str) -> bool {
    let lowered = property.to_lowercase();
    CAPTION_NAME_HINTS.iter().any(|hint| lowered.contains(hint))
}

/// Character length of whichever example value the stat carries.
fn sample_chars(stat: &PropertyStat) -> Option<usize> {
    stat.sample
        .as_ref()
        .or_else(|| stat.values.first())
        .and_then(|value| value.as_str())
        .map(str::chars)
        .map(Iterator::count)
}

/// Decide what appearance channel a property can drive.
///
/// Pure, and separated from the kglite call, so the `approx` path has a unit
/// test that needs no 200 000-node graph: a synthetic stat is enough to pin
/// that a sampled distinct-count never becomes a categorical palette. Colouring
/// by a property whose value *set* is a lower bound gives some nodes no colour
/// at all, silently.
pub(crate) fn classify(
    name: &str,
    value_type: &str,
    unique: usize,
    non_null: usize,
    approx: bool,
) -> AppearanceRole {
    // kglite's canonical identity columns. Numeric or short-stringed they may
    // be, but "size the nodes by their id" is a channel that encodes nothing,
    // and `type` is constant within a type by construction. Named from
    // `CANONICAL_NODE_COLUMNS` rather than guessed.
    if CANONICAL_NODE_COLUMNS.contains(&name) {
        return AppearanceRole::Unsuitable;
    }
    if non_null == 0 || unique <= 1 {
        // A property every node shares, or none has, draws one flat colour.
        return AppearanceRole::Unsuitable;
    }
    // The exact strings kglite emits, observed from `compute_property_stats`
    // against the fixture rather than inferred from the `Value` variant names:
    // the columnar types come back capitalised and the canonical ones do not.
    let numeric = matches!(value_type, "Int64" | "Float64" | "uniqueid");
    if numeric {
        return AppearanceRole::Numeric;
    }
    if !approx && unique <= MAX_DISTINCT_VALUES {
        return AppearanceRole::Categorical;
    }
    AppearanceRole::Unsuitable
}

/// One node's stored properties.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct NodeDetail {
    pub protocol_version: u32,
    pub slot: u32,
    pub node_id: u32,
    pub node_type: String,
    pub title: String,
    /// `(key, value)` pairs, sorted by key. Sorted rather than in storage
    /// order: a properties panel that reorders itself between two nodes of the
    /// same type is unreadable.
    #[ts(type = "[string, unknown][]")]
    pub properties: Vec<(String, serde_json::Value)>,
}

/// Read one node's properties.
pub fn node_detail(graph: &DirGraph, slot: u32, node_id: u32) -> Result<NodeDetail, CoreError> {
    // The arena guard the disk backend needs for materialised reads; a no-op
    // on memory and mapped graphs.
    let _guard = graph.begin_read_pass();
    let view = graph
        .node_view(NodeIndex::new(node_id as usize))
        .ok_or_else(|| CoreError::Request(format!("node {node_id} is not in this graph")))?;

    let mut properties: Vec<(String, serde_json::Value)> = view
        .property_pairs_named(&graph.interner)
        .into_iter()
        .map(|(key, value)| (key, value_to_json(&value)))
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(NodeDetail {
        protocol_version: PROTOCOL_VERSION,
        slot,
        node_id,
        node_type: view.node_type_str(&graph.interner).to_string(),
        title: value_to_display(&view.title()),
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sampled_string_property_never_becomes_a_colour_palette() {
        // The synthetic-stats path D12 requires a test for: 12 distinct values
        // is inside the categorical ceiling, but the count is a LOWER BOUND, so
        // colouring by it would leave the values nobody sampled uncoloured.
        assert_eq!(
            classify("bio", "String", 12, 500_000, true),
            AppearanceRole::Unsuitable
        );
        assert_eq!(
            classify("bio", "String", 12, 500, false),
            AppearanceRole::Categorical,
            "the same shape scanned exhaustively is fine"
        );
    }

    #[test]
    fn a_sampled_numeric_property_is_still_usable_for_size() {
        // Size-by reads each node's own value, not the distinct set, so
        // sampling the *statistics* does not make the channel wrong — only the
        // range hint is approximate, and that is what `approx` labels.
        assert_eq!(
            classify("score", "Float64", 900_000, 1_000_000, true),
            AppearanceRole::Numeric
        );
    }

    #[test]
    fn constant_and_empty_properties_are_not_offered() {
        assert_eq!(
            classify("flag", "Int64", 1, 100, false),
            AppearanceRole::Unsuitable
        );
        assert_eq!(
            classify("bio", "String", 0, 0, false),
            AppearanceRole::Unsuitable
        );
    }

    #[test]
    fn kglites_identity_columns_are_never_appearance_channels() {
        // `id` is numeric and `type` is a string with one value per type, so
        // both would otherwise pass the shape tests and appear in a menu where
        // they encode nothing.
        for name in CANONICAL_NODE_COLUMNS {
            assert_eq!(
                classify(name, "Int64", 60, 60, false),
                AppearanceRole::Unsuitable,
                "{name} must not be offered as an appearance channel"
            );
        }
    }

    /// A string stat, shaped by the two numbers the ranking actually reads.
    fn string_stat(name: &str, non_null: u32, unique: u32, sample: &str) -> PropertyStat {
        PropertyStat {
            name: name.to_string(),
            value_type: "String".to_string(),
            non_null,
            unique,
            values: Vec::new(),
            sample: Some(serde_json::Value::String(sample.to_string())),
            approx: unique as usize >= MAX_DISTINCT_VALUES,
            role: AppearanceRole::Unsuitable,
        }
    }

    /// A title good enough to keep — well covered and near-unique.
    fn good_title(count: u32) -> PropertyStat {
        let mut stat = string_stat("title", count, count, "EKOFISK");
        stat.value_type = "str".to_string();
        stat
    }

    /// A title that names nothing: 8 620 rows, eight distinct values. Sodir's
    /// `WellboreCasing`, which is the shape the feature exists for.
    fn useless_title(count: u32) -> PropertyStat {
        let mut stat = string_stat("title", count, 8, "CASING");
        stat.value_type = "str".to_string();
        stat.approx = false;
        stat
    }

    #[test]
    fn a_good_title_is_never_replaced() {
        // The rule sodir bought. `Field` is titled EKOFISK and carries a
        // `wlbName` of 25/1-4; ranking on "which column looks like a name"
        // offered the code and would have put it on every chip on the screen.
        let stats = vec![
            good_title(144),
            string_stat("wlbName", 140, 140, "25/1-4"),
            string_stat("fldOwnerName", 126, 126, "SLEIPNER VEST UNIT"),
        ];
        assert_eq!(caption_candidate(&stats, 144), None);
    }

    #[test]
    fn a_useless_title_is_replaced_by_the_name_column_beside_it() {
        let stats = vec![
            useless_title(8_620),
            string_stat("wlbName", 8_620, MAX_DISTINCT_VALUES as u32, "34/10-A-1 H"),
        ];
        assert_eq!(caption_candidate(&stats, 8_620).as_deref(), Some("wlbName"));

        // …and only by a column that IS a name. The same useless title with
        // nothing hinted beside it stays, because a column this heuristic
        // merely likes must not replace what the graph's author chose.
        let unhinted = vec![
            useless_title(8_620),
            string_stat("wlbCode", 8_620, MAX_DISTINCT_VALUES as u32, "AB-12"),
        ];
        assert_eq!(caption_candidate(&unhinted, 8_620), None);
    }

    #[test]
    fn a_caption_must_cover_the_type_distinguish_its_nodes_and_fit_a_chip() {
        // Each of these is a hinted name column beside a title that needs
        // replacing, and each is rejected for exactly one reason — so a rule
        // that stopped firing shows up here as one failing assert rather than
        // as a silently worse screen.
        let with = |stat: PropertyStat| vec![useless_title(100), stat];
        assert_eq!(
            caption_candidate(&with(string_stat("fldName", 40, 40, "Troll")), 100),
            None,
            "40% coverage is a name for a subset, not for the type"
        );
        assert_eq!(
            caption_candidate(&with(string_stat("statusName", 100, 3, "PRODUCING")), 100),
            None,
            "three distinct values is a category — that is what colour-by is for"
        );
        assert_eq!(
            caption_candidate(
                &with(string_stat("cmpLongName", 100, 100, &"x".repeat(80))),
                100
            ),
            None,
            "an 80-character value is a description, and a label chip holds ~18"
        );
        // And the same stat inside every limit is taken, so the three asserts
        // above are testing the limits rather than a function that says no.
        assert_eq!(
            caption_candidate(&with(string_stat("fldName", 100, 100, "Troll")), 100).as_deref(),
            Some("fldName")
        );
    }

    #[test]
    fn a_capped_distinct_count_still_reads_as_near_unique() {
        // kglite stops enumerating at MAX_DISTINCT_VALUES and sets `approx`, so
        // a genuinely unique name column reports exactly the cap. Reading that
        // as "only 32 distinct values, therefore a category" would reject every
        // caption on any type larger than the cap — i.e. all of them.
        let stat = string_stat(
            "wlbWellboreName",
            5_000,
            MAX_DISTINCT_VALUES as u32,
            "31/2-1",
        );
        assert!(stat.approx, "the fixture must exercise the approx branch");
        assert_eq!(
            caption_candidate(&[useless_title(5_000), stat], 5_000).as_deref(),
            Some("wlbWellboreName")
        );
    }

    #[test]
    fn identity_columns_and_numbers_are_never_captions() {
        // A type with no title at all is the other way in: nothing gates the
        // ranking, and `id` / `type` must still be refused.
        let mut numeric = string_stat("fieldNameId", 100, 100, "1");
        numeric.value_type = "Int64".to_string();
        assert_eq!(
            caption_candidate(&[numeric], 100),
            None,
            "a number is not a name, however well populated"
        );
        let mut identity = string_stat("type", 100, 100, "Field");
        identity.value_type = "str".to_string();
        assert_eq!(caption_candidate(&[identity], 100), None);
    }

    #[test]
    fn a_high_cardinality_string_is_not_categorical() {
        assert_eq!(
            classify("bio", "String", MAX_DISTINCT_VALUES + 1, 1_000, false),
            AppearanceRole::Unsuitable
        );
        assert_eq!(
            classify("bio", "String", MAX_DISTINCT_VALUES, 1_000, false),
            AppearanceRole::Categorical
        );
    }
}
