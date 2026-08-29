//! The label collision grid, ported from the app's overlay (plan D13).
//!
//! Mirrors `chooseLabels` / `estimateWidth` / `NUDGES` in
//! `frontend/src/labels.ts`, including the deterministic tie-break: heaviest
//! first, and an exact tie goes to the **lower slot**. In the browser that
//! stability is what stops the overlay flickering between two frames that are
//! otherwise identical; here it is what makes the golden SVG a baseline rather
//! than a sample.
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
    /// A supporting type's name is drawn quieter, matching its circle.
    pub dimmed: bool,
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
pub fn estimate_width(spec: &LabelSpec) -> f64 {
    let count_chars = super::encoding::group_thousands(spec.weight)
        .chars()
        .count();
    14.0 + spec.text.chars().count() as f64 * 6.9
        + 6.0
        + count_chars as f64 * 6.3
        + spec.badges.len() as f64 * 34.0
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

fn columns_for(x: f64, width: f64) -> (i64, i64) {
    (
        ((x - width / 2.0) / CELL_WIDTH).floor() as i64,
        ((x + width / 2.0) / CELL_WIDTH).floor() as i64,
    )
}

fn is_free(taken: &std::collections::HashSet<(i64, i64)>, from: i64, to: i64, row: i64) -> bool {
    (from..=to).all(|column| !taken.contains(&(column, row)))
}

fn claim(taken: &mut std::collections::HashSet<(i64, i64)>, from: i64, to: i64, row: i64) {
    for column in from..=to {
        taken.insert((column, row));
    }
}

/// Place labels, at most one per screen cell.
///
/// Mirrors `chooseLabels` in `frontend/src/labels.ts`, `place_all` included.
///
/// `place_all` is the meta-graph's mode, and the meta-graph IS its labels — a
/// type node with no name on it is a dot, which is precisely the entry screen a
/// user reported as useless. So there, a label that loses its cell is *nudged*
/// into a free neighbour rather than dropped, and if nothing near it is free it
/// is drawn overlapping: a hundred type names crowding each other is a legible
/// schema, and ninety-eight dots with sixty names is not. An instance slice
/// keeps dropping, because at the 5 000-node response bound "every label" is
/// not a picture at any density.
pub fn choose(specs: &[LabelSpec], place_all: bool) -> Vec<PlacedLabel> {
    // Sorted rather than compared in place: the winner of a cell must not
    // depend on the caller's output order. Heaviest first, and an exact tie
    // goes to the lower slot — the one identifier that is stable across zooms,
    // expansions and reconnects.
    let mut ordered: Vec<&LabelSpec> = specs.iter().collect();
    ordered.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.slot.cmp(&b.slot)));

    let mut taken: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    let mut placed: Vec<PlacedLabel> = Vec::new();
    for spec in ordered {
        let (from, to) = columns_for(spec.x, estimate_width(spec));
        let row = (spec.y / CELL_HEIGHT).floor() as i64;
        if is_free(&taken, from, to, row) {
            claim(&mut taken, from, to, row);
            placed.push(PlacedLabel {
                slot: spec.slot,
                x: spec.x,
                y: spec.y,
            });
            continue;
        }
        if !place_all {
            continue;
        }
        let nudge = NUDGES
            .iter()
            .find(|(dx, dy)| is_free(&taken, from + dx, to + dx, row + dy));
        let Some((dx, dy)) = nudge else {
            // Every neighbour is spoken for. Drawing it anyway is the
            // deliberate choice: an unnamed type node carries no information at
            // all.
            placed.push(PlacedLabel {
                slot: spec.slot,
                x: spec.x,
                y: spec.y,
            });
            continue;
        };
        claim(&mut taken, from + dx, to + dx, row + dy);
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
            dimmed: false,
            x,
            y,
        }
    }

    #[test]
    fn an_exact_tie_goes_to_the_lower_slot() {
        // The tie-break `labels.ts` calls out by name. Without it, which of two
        // equally weighted labels survives a shared cell depends on input
        // order, and the golden SVG stops being a baseline.
        let a = choose(&[spec(9, 100, 60.0, 15.0), spec(4, 100, 60.0, 15.0)], false);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].slot, 4);

        let reversed = choose(&[spec(4, 100, 60.0, 15.0), spec(9, 100, 60.0, 15.0)], false);
        assert_eq!(reversed, a, "input order must not decide the winner");
    }

    #[test]
    fn the_heavier_label_wins_a_contested_cell() {
        let placed = choose(&[spec(0, 5, 60.0, 15.0), spec(1, 5_000, 60.0, 15.0)], false);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].slot, 1);
    }

    #[test]
    fn place_all_nudges_the_loser_instead_of_dropping_it() {
        let placed = choose(&[spec(0, 5, 60.0, 15.0), spec(1, 5_000, 60.0, 15.0)], true);
        assert_eq!(placed.len(), 2, "the meta-graph names every type");
        let loser = placed.iter().find(|p| p.slot == 0).expect("slot 0 placed");
        assert_eq!(
            (loser.x, loser.y),
            (60.0, 15.0 - CELL_HEIGHT),
            "the first nudge is one row up"
        );
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
        let placed = choose(&[wide, spec(1, 1, 200.0 + CELL_WIDTH, 15.0)], false);
        assert_eq!(placed.len(), 1, "the neighbour's cell was already claimed");
        assert_eq!(placed[0].slot, 0);
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
