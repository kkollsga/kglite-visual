//! The label collision grid, ported from the app's overlay (plan D13).
//!
//! Mirrors `chooseLabels` / `estimateWidth` / `NUDGES` in
//! `frontend/src/labels.ts`, including the deterministic tie-break: heaviest
//! first, and an exact tie goes to the **lower slot**. In the browser that
//! stability is what stops the overlay flickering between two frames that are
//! otherwise identical; here it is what makes the golden SVG a baseline rather
//! than a sample.
//!
//! **Three deliberate divergences, all because an image has no camera.**
//! [`budget`] caps how many labels are placed at all (round 2),
//! [`LabelSpec::degree`] orders the survivors (round 2), and [`choose`] thins a
//! `place_all` label whose region is full instead of drawing it on top of its
//! neighbours (round 4). The app needs none of the three: a contested cell
//! there is resolved by the user zooming, and the overlay runs over *sampled*
//! points that change every frame, so a hard cap would make a name appear and
//! vanish on a pan. Degree is also not a quantity the overlay holds —
//! `View.links` is an array of point indices, not slots — so mirroring it would
//! mean giving the browser a second edge index for a tie-break it never
//! reaches. What still mirrors constant-for-constant is [`estimate_width`],
//! [`columns_for`] and [`rows_for`], which are the ones that must agree or the
//! two sides resolve collisions differently.
//!
//! The estimate-don't-measure rule ports too, and for a second reason. In the
//! app, measuring means laying out every candidate on every camera event, which
//! the overlay's whole design forbids. Here there is no camera and no DOM at
//! all: an SVG emitter has no text metrics, so an estimate is the only width
//! available — and it had better be the *same* estimate, or the two sides
//! resolve collisions differently and the images stop matching.

/// Screen-space cell size, in pixels. Roughly one label's footprint.
///
/// Mirrors `CELL_WIDTH` / `CELL_HEIGHT` in `frontend/src/labels.ts`.
pub const CELL_WIDTH: f64 = 130.0;
pub const CELL_HEIGHT: f64 = 30.0;

/// One candidate label.
///
/// Mirrors `LabelSpec` + the candidate shape `chooseLabels` takes in
/// `frontend/src/labels.ts`.
#[derive(Debug, Clone)]
pub struct LabelSpec {
    pub slot: u32,
    pub text: String,
    /// Capability flags (`ts`/`geo`/`loc`/`vec`) rendered as small badges.
    pub badges: Vec<String>,
    /// Bigger wins a screen cell. Node count, in practice.
    pub weight: u64,
    /// Links this node has in *this picture* — the tie-break under `weight`.
    ///
    /// **What decides "top N" on an instance slice** (P11 round 2). Every
    /// instance node carries `weight: 1`, so before this the sort fell straight
    /// through to the slot id and "the labels that fit" meant "the ones that
    /// happened to arrive first". Degree is the picture's own answer to which
    /// nodes a reader is trying to find: the hub a fan hangs off, not the
    /// forty-third leaf on it.
    pub degree: u32,
    /// Whether the count chip is drawn at all.
    ///
    /// **False for a plain instance node, whose count is always 1** (P11). The
    /// P9 renders put a grey `1` beside every wellbore name in the picture — a
    /// number with no information in it, repeated three hundred times, eating
    /// the width the name needed. A count chip appears where there is a count:
    /// on a type node, on an aggregate, and on anything else standing for more
    /// than one thing.
    pub show_count: bool,
    /// A supporting type's name is drawn quieter, matching its circle.
    pub dimmed: bool,
    /// This label is placed before every other and is never dropped.
    ///
    /// For the one node the picture is *about* — an ego layout's centre, an
    /// aggregate glyph whose count is the only thing making it honest. Weight
    /// cannot express that: an ego centre's weight is 1, and inflating it to
    /// win a cell would put a fabricated number in the chip.
    pub pinned: bool,
    /// Centre of the label, in canvas pixels.
    pub x: f64,
    /// Top of the label — below the circle it names, never on top of it: a
    /// label centred on its point hides the size that encodes the count.
    pub y: f64,
}

/// A label that won a place, with the position it won.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedLabel {
    pub slot: u32,
    pub x: f64,
    pub y: f64,
}

/// A label's width in pixels, estimated from its content.
///
/// Mirrors `estimateWidth` in `frontend/src/labels.ts`, constant for constant:
/// 14 px of padding, ~6.9 px per character of the name, a 6 px gap, 6.3 px per
/// character of the tabular-nums count, and 34 px per badge. The estimate only
/// has to be good enough to keep a 200 px name from claiming one 130 px cell
/// and then sitting on top of two neighbours'.
///
/// The gap and the count term are charged only when the chip has a count in it
/// ([`LabelSpec::show_count`]) — an estimate that reserved room for a number
/// nobody drew would thin the labels on an instance slice for nothing.
pub fn estimate_width(spec: &LabelSpec) -> f64 {
    let count = if spec.show_count {
        6.0 + super::encoding::group_thousands(spec.weight)
            .chars()
            .count() as f64
            * 6.3
    } else {
        0.0
    };
    14.0 + spec.text.chars().count() as f64 * 6.9 + count + spec.badges.len() as f64 * 34.0
}

/// Cells one legible label needs, counting its neighbours.
///
/// A chip is ~1.6 cells wide at sodir's type names, and a row of chips with the
/// rows above and below also full is a wall of text rather than a labelled
/// graph. Three cells per label is the density at which the 1600x1000
/// meta-graph render reads, measured on that image: it puts the budget at 116
/// for 98 types (no thinning) and at 24 for the same graph at 800x500, where
/// round 1 forced all 98 into about 96 cells and produced an image the
/// coordinator called unusable.
const CELLS_PER_LABEL: usize = 3;

/// Rows held back for the status block.
///
/// A fixed allowance rather than the block's real height, and deliberately: the
/// budget decides whether a "not every name is shown" line is added, that line
/// changes the block's height, and a budget derived from the height would be
/// deciding its own input. Four rows is the block at its tallest — path, tier,
/// counts, fold line, banner — rounded up.
const STATUS_ROWS: usize = 4;

/// How many labels a `width` x `height` canvas can hold legibly.
///
/// **The failure this answers has a picture.** The meta-graph promises every
/// type is named, and at 800x500 that meant 98 chips forced into roughly 96
/// cells: names on top of names, nothing readable, and no way for a reader to
/// tell which name belonged to which circle. A promise a canvas cannot keep is
/// not honesty, it is noise — the honest version keeps the names a reader can
/// use and *says* how many it dropped, which is what `super::render::draw`
/// adds to the status block when this bites.
pub fn budget(width: u32, height: u32) -> usize {
    let columns = (f64::from(width) / CELL_WIDTH).floor().max(1.0) as usize;
    let rows = (f64::from(height) / CELL_HEIGHT).floor().max(1.0) as usize;
    (columns * rows.saturating_sub(STATUS_ROWS) / CELLS_PER_LABEL).max(1)
}

/// Cells a displaced label will try, in order, before it gives up and overlaps.
///
/// Mirrors `NUDGES` in `frontend/src/labels.ts`. Vertical first and only ±2
/// rows out, because a label has to stay recognisably attached to the circle it
/// names. Fixed order, so the choice is a function of the input and nothing
/// else.
const NUDGES: [(i64, i64); 10] = [
    (0, -1),
    (0, 1),
    (0, -2),
    (0, 2),
    (-1, 0),
    (1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// Height of a drawn chip, in pixels. Mirrors `CHIP_HEIGHT` in [`super::svg`]
/// and `.kglv-label`'s box in `frontend/src/styles.css`. It is *smaller* than
/// [`CELL_HEIGHT`], which is the whole reason [`rows_for`] exists.
const CHIP_HEIGHT: f64 = 20.0;

fn columns_for(x: f64, width: f64) -> (i64, i64) {
    (
        ((x - width / 2.0) / CELL_WIDTH).floor() as i64,
        ((x + width / 2.0) / CELL_WIDTH).floor() as i64,
    )
}

/// Rows a chip drawn at `y` covers.
///
/// **The P4b fix's other axis, and the same defect** (P11 round 4). A chip is
/// 20 px tall and a row is 30, so two chips 15 px apart in y land in *different*
/// rows: the grid seats both, and the emitter draws them on top of each other.
/// Measured on round 3's portfolio, that is where nearly all the overlap came
/// from — 26 of 53 chips in the bipartite render and 28 of 92 in the licensee
/// render overlapped another chip, in a mode with no stacking policy anywhere
/// near it, and every such pair was in an adjacent row. A chip now reserves
/// every row it covers, exactly as [`columns_for`] already makes it reserve
/// every column.
///
/// The epsilon keeps a chip whose bottom edge lands exactly on a row boundary
/// out of the row below, which it touches with zero area.
fn rows_for(y: f64) -> (i64, i64) {
    (
        (y / CELL_HEIGHT).floor() as i64,
        ((y + CHIP_HEIGHT - 1e-9) / CELL_HEIGHT).floor() as i64,
    )
}

fn is_free(
    taken: &std::collections::HashSet<(i64, i64)>,
    from: i64,
    to: i64,
    rows: (i64, i64),
) -> bool {
    (from..=to).all(|column| (rows.0..=rows.1).all(|row| !taken.contains(&(column, row))))
}

fn claim(taken: &mut std::collections::HashSet<(i64, i64)>, from: i64, to: i64, rows: (i64, i64)) {
    for column in from..=to {
        for row in rows.0..=rows.1 {
            taken.insert((column, row));
        }
    }
}

/// Place labels, at most one per screen cell.
///
/// Mirrors `chooseLabels` in `frontend/src/labels.ts`, `place_all` included.
///
/// `place_all` is the meta-graph's mode, and the meta-graph IS its labels — a
/// type node with no name on it is a dot, which is precisely the entry screen a
/// user reported as useless. So there, a label that loses its cell is *nudged*
/// into a free neighbour rather than dropped: it gets its home cell and the ten
/// [`NUDGES`] around it, eleven chances an instance slice does not get.
///
/// **Only once that whole region is full does it give up its name** (P11 round
/// 4). Round 2 drew it on top of its neighbours instead, on the argument that a
/// crowded name beats an anonymous dot, and round 3 measured what that costs on
/// sodir: two piles, one of about seven names in the Discovery island, where no
/// reader can tell which name belongs to which circle. Seven illegible names
/// carry less than two legible ones and five dots — and the sort below decides
/// which two, so what is lost is the small fry and never the hub. `super::draw`
/// puts the number of names it dropped in the status block, because a picture
/// that quietly stops naming things is the D5 failure with a different subject.
///
/// An instance slice still drops on the first collision, because at the 5 000-
/// node response bound "every label" is not a picture at any density.
///
/// `budget` caps how many labels may be placed at all, however much room the
/// grid finds — see [`budget`]. A pinned label is never dropped by it: the
/// aggregate glyph's count and the ego centre's name are the two labels a
/// picture cannot be read without, and a cap that silenced them would be
/// trading legibility for honesty rather than buying both.
pub fn choose(specs: &[LabelSpec], place_all: bool, budget: usize) -> Vec<PlacedLabel> {
    // Sorted rather than compared in place: the winner of a cell must not
    // depend on the caller's output order. Pinned first, then heaviest, then
    // best connected, and an exact tie goes to the lower slot — the one
    // identifier that is stable across zooms, expansions and reconnects.
    //
    // Degree sits under weight rather than beside it because the two answer
    // different graphs: a type node's weight is its member count, and every
    // instance node's is 1, so degree is what actually orders an instance
    // slice and weight is what orders a meta-graph.
    let mut ordered: Vec<&LabelSpec> = specs.iter().collect();
    ordered.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.weight.cmp(&a.weight))
            .then_with(|| b.degree.cmp(&a.degree))
            .then_with(|| a.slot.cmp(&b.slot))
    });

    let mut taken: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    let mut placed: Vec<PlacedLabel> = Vec::new();
    for spec in ordered {
        if placed.len() >= budget && !spec.pinned {
            break;
        }
        let (from, to) = columns_for(spec.x, estimate_width(spec));
        let rows = rows_for(spec.y);
        if is_free(&taken, from, to, rows) {
            claim(&mut taken, from, to, rows);
            placed.push(PlacedLabel {
                slot: spec.slot,
                x: spec.x,
                y: spec.y,
            });
            continue;
        }
        // A pinned label takes the meta-graph's branch whatever the mode is:
        // it is nudged, and drawn overlapping if nothing near it is free. An
        // aggregate glyph without its count is a circle standing for forty
        // nodes with nothing on the picture saying so, which is the honesty
        // failure D5 is about.
        if !place_all && !spec.pinned {
            continue;
        }
        let nudge = NUDGES
            .iter()
            .find(|(dx, dy)| is_free(&taken, from + dx, to + dx, (rows.0 + dy, rows.1 + dy)));
        let Some((dx, dy)) = nudge else {
            // Every cell this label can reach is spoken for, so it keeps its
            // circle and loses its name. A pin is the exception and stays: an
            // aggregate glyph's count and an ego centre's name are the two
            // labels a picture cannot be read without, and dropping one would
            // trade this honesty failure for that one.
            if !spec.pinned {
                continue;
            }
            placed.push(PlacedLabel {
                slot: spec.slot,
                x: spec.x,
                y: spec.y,
            });
            continue;
        };
        claim(&mut taken, from + dx, to + dx, (rows.0 + dy, rows.1 + dy));
        placed.push(PlacedLabel {
            slot: spec.slot,
            x: spec.x + *dx as f64 * CELL_WIDTH,
            y: spec.y + *dy as f64 * CELL_HEIGHT,
        });
    }
    placed.sort_by_key(|p| p.slot);
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(slot: u32, weight: u64, x: f64, y: f64) -> LabelSpec {
        LabelSpec {
            slot,
            text: "T".to_string(),
            badges: Vec::new(),
            weight,
            degree: 0,
            show_count: true,
            dimmed: false,
            pinned: false,
            x,
            y,
        }
    }

    #[test]
    fn an_exact_tie_goes_to_the_lower_slot() {
        // The tie-break `labels.ts` calls out by name. Without it, which of two
        // equally weighted labels survives a shared cell depends on input
        // order, and the golden SVG stops being a baseline.
        let a = choose(
            &[spec(9, 100, 60.0, 15.0), spec(4, 100, 60.0, 15.0)],
            false,
            usize::MAX,
        );
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].slot, 4);

        let reversed = choose(
            &[spec(4, 100, 60.0, 15.0), spec(9, 100, 60.0, 15.0)],
            false,
            usize::MAX,
        );
        assert_eq!(reversed, a, "input order must not decide the winner");
    }

    #[test]
    fn the_heavier_label_wins_a_contested_cell() {
        let placed = choose(
            &[spec(0, 5, 60.0, 15.0), spec(1, 5_000, 60.0, 15.0)],
            false,
            usize::MAX,
        );
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].slot, 1);
    }

    #[test]
    fn place_all_nudges_the_loser_instead_of_dropping_it() {
        let placed = choose(
            &[spec(0, 5, 60.0, 15.0), spec(1, 5_000, 60.0, 15.0)],
            true,
            usize::MAX,
        );
        assert_eq!(placed.len(), 2, "the meta-graph names every type it can");
        let loser = placed.iter().find(|p| p.slot == 0).expect("slot 0 placed");
        assert_eq!(
            (loser.x, loser.y),
            (60.0, 15.0 - 2.0 * CELL_HEIGHT),
            "the first nudge with room for a whole chip is two rows up"
        );
    }

    #[test]
    fn a_full_region_thins_instead_of_stacking_chips() {
        // Ten names on one point. `place_all` seats the five the grid can fit —
        // the home cell and the four NUDGES that clear a whole chip — and round
        // 2 drew the other five at identical coordinates, which is the pile
        // round 3 measured in the Discovery island. They are dropped now.
        let specs: Vec<LabelSpec> = (0..10).map(|i| spec(i, u64::from(i), 60.0, 15.0)).collect();
        let placed = choose(&specs, true, usize::MAX);
        assert_eq!(
            placed.len(),
            5,
            "the home cell and the four nudges with room for a chip"
        );
        // …and the survivors are the heaviest, not whoever arrived last: a hub
        // type's name outranks the small fry sharing its cell.
        let mut slots: Vec<u32> = placed.iter().map(|p| p.slot).collect();
        slots.sort_unstable();
        assert_eq!(slots, vec![5, 6, 7, 8, 9], "the five heaviest");
    }

    #[test]
    fn a_pinned_label_survives_a_region_that_is_already_full() {
        // Every one of them pinned, so the thinning is the only thing that
        // could drop one — several folded fans landing in one corner is exactly
        // the arrangement that reaches this branch, and a wedge without its
        // count is a circle standing for forty nodes and saying nothing.
        let specs: Vec<LabelSpec> = (0..10)
            .map(|i| LabelSpec {
                pinned: true,
                ..spec(i, u64::from(i), 60.0, 15.0)
            })
            .collect();
        let placed = choose(&specs, true, usize::MAX);
        assert_eq!(placed.len(), 10, "a pin outlives a full region");
    }

    #[test]
    fn a_tall_label_reserves_every_row_it_covers() {
        // The P4b fix's other axis. Two chips 15 px apart in y are in different
        // 30 px rows and 20 px tall, so before `rows_for` the grid seated both
        // and the emitter drew them on top of each other — which is where round
        // 3's overlap count came from, in every mode, meta-graph or not.
        let low = spec(0, 1_000, 60.0, 29.0);
        let high = spec(1, 1, 60.0, 44.0);
        assert_ne!(
            (low.y / CELL_HEIGHT).floor(),
            (high.y / CELL_HEIGHT).floor(),
            "the two must be in different rows, or this tests nothing"
        );
        assert!(
            high.y < low.y + CHIP_HEIGHT,
            "…and they must genuinely overlap on the canvas"
        );
        let placed = choose(&[low, high], false, usize::MAX);
        assert_eq!(
            placed.len(),
            1,
            "the second chip would have been drawn on the first"
        );
        assert_eq!(placed[0].slot, 0);
    }

    #[test]
    fn a_wide_label_reserves_every_cell_it_covers() {
        // The P4b fix, ported: a fixed one-cell reservation let a 200 px name
        // claim one 130 px cell and then sit on top of two neighbours'.
        let wide = LabelSpec {
            text: "AnExtremelyLongTypeNameIndeed".to_string(),
            weight: 1_000_000,
            ..spec(0, 1_000_000, 200.0, 15.0)
        };
        assert!(estimate_width(&wide) > 2.0 * CELL_WIDTH);
        let placed = choose(
            &[wide, spec(1, 1, 200.0 + CELL_WIDTH, 15.0)],
            false,
            usize::MAX,
        );
        assert_eq!(placed.len(), 1, "the neighbour's cell was already claimed");
        assert_eq!(placed[0].slot, 0);
    }

    #[test]
    fn a_pinned_label_survives_a_cell_it_would_otherwise_lose() {
        // R1 for the pin: without it the ego centre and the aggregate glyphs —
        // the two labels a picture cannot be read without — are exactly the
        // lightest ones, and the grid drops the lightest first.
        let heavy = spec(1, 5_000, 60.0, 15.0);
        let pinned = LabelSpec {
            pinned: true,
            ..spec(0, 1, 60.0, 15.0)
        };
        let placed = choose(&[heavy.clone(), pinned], false, usize::MAX);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].slot, 0, "the pin outranks the weight");
        // …and without the pin the same pair resolves the other way, so the
        // assertion above is testing the pin and not the sort's tie-break.
        let unpinned = choose(&[heavy, spec(0, 1, 60.0, 15.0)], false, usize::MAX);
        assert_eq!(unpinned.len(), 1);
        assert_eq!(unpinned[0].slot, 1);
    }

    #[test]
    fn a_canvas_gets_the_labels_it_can_hold_and_no_more() {
        // The picture that bought it: 98 type names forced into a 800x500
        // canvas with room for about 96 cells. Asserted against the two canvas
        // sizes the portfolio uses, so a change to either constant shows up as
        // a change to the sizes people actually render.
        assert!(
            budget(1_600, 1_000) >= 98,
            "the chat-size canvas names all 98 sodir types: {}",
            budget(1_600, 1_000)
        );
        assert!(
            budget(800, 500) < 40,
            "the thumbnail cannot, and must not pretend: {}",
            budget(800, 500)
        );
        assert!(budget(200, 200) >= 1, "a tiny canvas still names something");
    }

    #[test]
    fn the_budget_drops_the_lightest_and_never_a_pinned_label() {
        // Ten candidates, well spread so the grid itself would place every one:
        // whatever is missing was dropped by the budget and by nothing else.
        let mut specs: Vec<LabelSpec> = (0..10)
            .map(|i| spec(i, u64::from(i), f64::from(i) * 400.0, 15.0))
            .collect();
        specs[0].pinned = true;
        let placed = choose(&specs, false, 3);
        // A pin spends a cell like anything else — the budget is the canvas's
        // capacity, and a label that ignored it would be drawn on top of one
        // that did not.
        assert_eq!(placed.len(), 3, "the budget is a capacity, not a quota");
        let slots: Vec<u32> = placed.iter().map(|p| p.slot).collect();
        assert!(slots.contains(&0), "a pinned label outlives the budget");
        for heavy in [9, 8] {
            assert!(slots.contains(&heavy), "the heaviest survive: {slots:?}");
        }
    }

    #[test]
    fn degree_orders_labels_that_weigh_the_same() {
        // Every instance node weighs 1, so without this the survivors are
        // whichever slots happened to come first.
        let mut specs: Vec<LabelSpec> = (0..4)
            .map(|i| spec(i, 1, f64::from(i) * 400.0, 15.0))
            .collect();
        specs[3].degree = 40;
        let placed = choose(&specs, false, 1);
        assert_eq!(placed.len(), 1);
        assert_eq!(
            placed[0].slot, 3,
            "the hub is named, not the lowest slot number"
        );
    }

    #[test]
    fn a_chip_with_no_count_reserves_no_room_for_one() {
        let with = spec(0, 1, 0.0, 0.0);
        let without = LabelSpec {
            show_count: false,
            ..spec(0, 1, 0.0, 0.0)
        };
        assert!((estimate_width(&with) - estimate_width(&without) - (6.0 + 6.3)).abs() < 1e-9);
    }

    #[test]
    fn the_width_estimate_matches_the_typescript_constants() {
        // 14 padding + 3 chars * 6.9 + 6 gap + "1,000" (5 chars) * 6.3 + 1 badge * 34
        let s = LabelSpec {
            text: "Abc".to_string(),
            badges: vec!["geo".to_string()],
            ..spec(0, 1_000, 0.0, 0.0)
        };
        assert!((estimate_width(&s) - (14.0 + 20.7 + 6.0 + 31.5 + 34.0)).abs() < 1e-9);
    }
}
