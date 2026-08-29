//! Deterministic fixture layout — the server-supplied positions D2 requires.
//!
//! D2 settled that determinism comes from the *server* handing over
//! positions, because cosmos.gl's own `randomSeed` is init-only and its
//! simulation leaks nondeterminism three ways. So this module is the position
//! source, and the committed positions fixture is its output, not its input.
//!
//! **No transcendental functions, on purpose.** The obvious layout here is a
//! golden-angle spiral, and `sin`/`cos` are the platform's libm: two machines
//! can disagree in the last bit, which would make a committed positions
//! baseline fail on a machine that changed nothing. Every operation below is
//! an integer walk plus a multiply and a divide by a power of two, all exact
//! in `f32` — so the baseline means the same thing everywhere.
//!
//! This is a fixture layout, not a layout *feature*: the real-graph question
//! (cosmos.gl's GPU force layout vs a Rust-side ForceAtlas2) is Phase 4's,
//! and carries its own stop rule.

/// Lattice spacing between adjacent spiral cells, in graph-space units.
const SPACING: f32 = 140.0;

/// Jitter is quantized to 1/128 of a unit and bounded to ±32, so it is exact
/// in `f32` and cannot push a node into its neighbour's cell.
const JITTER_QUANTUM: f32 = 128.0;
const JITTER_STEPS: u64 = 8192;

/// Positions for `count` slots, as the `[x0, y0, x1, y1, …]` pair array the
/// wire and cosmos.gl's `setPointPositions` both take.
pub fn positions_for(count: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity(count as usize * 2);
    let mut walker = SpiralWalker::default();
    for slot in 0..count {
        let (cx, cy) = walker.next_cell();
        out.push(cx as f32 * SPACING + jitter(slot, 0));
        out.push(cy as f32 * SPACING + jitter(slot, 1));
    }
    out
}

/// A square spiral outward from the origin: (0,0), (1,0), (1,1), (0,1), …
///
/// Walked step by step rather than solved in closed form. A meta-graph has at
/// most a few thousand type nodes, so the walk is free, and a closed form
/// would trade that for an off-by-one nobody would notice until the baseline
/// moved.
#[derive(Debug)]
struct SpiralWalker {
    x: i32,
    y: i32,
    dir: usize,
    run_len: i32,
    run_left: i32,
    runs_at_len: u8,
    first: bool,
}

impl Default for SpiralWalker {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            dir: 0,
            run_len: 1,
            run_left: 1,
            runs_at_len: 0,
            first: true,
        }
    }
}

impl SpiralWalker {
    fn next_cell(&mut self) -> (i32, i32) {
        if self.first {
            self.first = false;
            return (0, 0);
        }
        const STEPS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
        let (dx, dy) = STEPS[self.dir];
        self.x += dx;
        self.y += dy;
        self.run_left -= 1;
        if self.run_left == 0 {
            self.dir = (self.dir + 1) % 4;
            // The square spiral's run lengths go 1,1,2,2,3,3,… — the length
            // grows only after two runs at the current length.
            self.runs_at_len += 1;
            if self.runs_at_len == 2 {
                self.runs_at_len = 0;
                self.run_len += 1;
            }
            self.run_left = self.run_len;
        }
        (self.x, self.y)
    }
}

/// A slot's deterministic offset within its cell, in ±32 graph-space units.
///
/// splitmix64 finalizer over `(slot, axis)`: a fixed bit mix, no RNG state, no
/// platform float behaviour. Reproducing it in another language is copying
/// four constants.
fn jitter(slot: u32, axis: u32) -> f32 {
    let mut z = (slot as u64) << 1 | axis as u64;
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let step = (z % JITTER_STEPS) as i64 - (JITTER_STEPS as i64 / 2);
    step as f32 / JITTER_QUANTUM
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spiral_visits_every_cell_of_the_first_two_rings() {
        let mut walker = SpiralWalker::default();
        let cells: Vec<(i32, i32)> = (0..9).map(|_| walker.next_cell()).collect();
        assert_eq!(
            cells,
            vec![
                (0, 0),
                (1, 0),
                (1, 1),
                (0, 1),
                (-1, 1),
                (-1, 0),
                (-1, -1),
                (0, -1),
                (1, -1),
            ]
        );
    }

    #[test]
    fn positions_are_pairwise_and_repeatable() {
        let a = positions_for(64);
        let b = positions_for(64);
        assert_eq!(a.len(), 128);
        assert_eq!(a, b, "the layout is a pure function of the slot count");
    }

    #[test]
    fn a_prefix_is_stable_when_more_slots_are_added() {
        // Expansion appends slots (D4). If adding a slot moved the ones
        // already on screen, every expand would look like a re-layout.
        let five = positions_for(5);
        let fifty = positions_for(50);
        assert_eq!(five, fifty[..10]);
    }

    #[test]
    fn jitter_stays_inside_its_cell() {
        for slot in 0..2000 {
            for axis in 0..2 {
                let j = jitter(slot, axis);
                assert!(j.abs() <= 32.0, "slot {slot} axis {axis} jittered {j}");
            }
        }
    }

    #[test]
    fn no_two_slots_share_a_position() {
        let positions = positions_for(512);
        let mut seen: Vec<(u32, u32)> = positions
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| (p[0].to_bits(), p[1].to_bits()))
            .collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "coincident points would stack labels");
    }
}
