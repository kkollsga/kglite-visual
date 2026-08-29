//! Messages that steer a view without changing what is in it (plan D14).
//!
//! Everything else on the wire answers a question about the graph. These three
//! answer a different one — *where should the user be looking* — and they exist
//! because P10 gave the view a second driver. A human already has a mouse for
//! all three; an agent asking "show them this" had no vocabulary at all.
//!
//! **They are commands, and they are broadcast.** Unlike a slice, they carry no
//! slot-space change: nothing is appended, nothing is tombstoned, and a client
//! that missed one is *stale*, not wrong. That is why they are separate message
//! types rather than fields on [`crate::view::GraphSliceMeta`] — an agent
//! highlighting a search result must not have to send an empty slice to do it,
//! and a slice must not silently move the camera.
//!
//! **Why three and not two.** The design named `focus` as the one message the
//! tool surface still needed. `set_appearance` turned out to need one too: the
//! colour-by / size-by choice lives entirely in the client (compiled getters
//! over values it fetched itself), so the server has no representation of it to
//! push and no way to reach it without a message of its own. Bundling it into
//! `Highlight` would have made one message mean two unrelated things.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::PROTOCOL_VERSION;

/// Zoom the camera to frame these slots.
///
/// Slots, not node ids — everything that names something in the view names it
/// by slot (see [`crate::request`]), and the client already holds the slot→
/// renderer-index map this needs.
///
/// An empty list means "frame the whole view": it is what an agent sends after
/// a collapse, and refusing it would leave the camera on a neighbourhood that
/// is no longer there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct Focus {
    pub protocol_version: u32,
    pub slots: Vec<u32>,
}

/// Which of the index-addressed interaction concepts a highlight drives (D7).
///
/// The other two — `hovered` and its `emphasized` neighbourhood — are
/// deliberately absent. They are a *cursor*, recomputed client-side from the
/// adjacency the renderer already holds, and a remote peer setting a hover
/// would fight the mouse of the human sitting in front of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum HighlightConcept {
    /// Search and query hits — a colour-array fill, so several may stand out at
    /// once without greying the rest of the graph. The default: "here is what
    /// I found" is what a caller who did not say means.
    #[default]
    Highlighted,
    /// What the selection panel is describing — the outline ring.
    Selected,
}

/// Mark these slots under one interaction concept.
///
/// Replaces that concept's set rather than adding to it: "these are the hits"
/// is the statement, and a caller that had to clear first would leave a frame
/// in which two answers were on screen together. An empty list clears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct Highlight {
    pub protocol_version: u32,
    pub slots: Vec<u32>,
    pub concept: HighlightConcept,
}

/// Drive the colour-by / size-by channels.
///
/// A property *name*, because that is what the client's own menus are keyed by
/// and what its property-statistics response describes. `None` clears the
/// channel back to the app's structural encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct Appearance {
    pub protocol_version: u32,
    pub color_by: Option<String>,
    pub size_by: Option<String>,
}

impl Focus {
    pub fn new(slots: Vec<u32>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            slots,
        }
    }
}

impl Highlight {
    pub fn new(slots: Vec<u32>, concept: HighlightConcept) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            slots,
            concept,
        }
    }
}

impl Appearance {
    pub fn new(color_by: Option<String>, size_by: Option<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            color_by,
            size_by,
        }
    }
}

/// Frame one steering command for the binary transport.
///
/// Beside [`crate::session::response_frames`] rather than inside it: those are
/// *answers* to a request the caller made, keyed by the request's own type,
/// while these are pushed to clients that asked for nothing. One encoder, two
/// origins.
pub fn control_frames(command: &Command) -> Vec<Vec<u8>> {
    use crate::protocol::{MessageType, ResponseEncoder};

    let mut enc = ResponseEncoder::new();
    let (msg_type, json) = match command {
        Command::Focus(focus) => (MessageType::Focus, json_of(focus)),
        Command::Highlight(highlight) => (MessageType::Highlight, json_of(highlight)),
        Command::Appearance(appearance) => (MessageType::Appearance, json_of(appearance)),
    };
    enc.push_json(msg_type, &json);
    enc.finish()
}

/// What a caller asks for, on the twin and through MCP.
///
/// Deserialize-only and separate from the wire messages above, for the reason
/// [`crate::request`] gives: these are things a caller *writes*, and
/// `protocol_version` is the server's to stamp. A caller who could set it could
/// tell every attached client it was speaking a version it is not.
#[derive(Debug, Clone, Deserialize)]
pub struct FocusRequest {
    /// Slots to frame. Empty frames the whole view.
    #[serde(default)]
    pub slots: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HighlightRequest {
    /// Slots to mark. Empty clears the concept.
    #[serde(default)]
    pub slots: Vec<u32>,
    #[serde(default)]
    pub concept: HighlightConcept,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppearanceRequest {
    /// Property driving the colour channel, or `null` to clear it.
    #[serde(default)]
    pub color_by: Option<String>,
    /// Property driving the size channel, or `null` to clear it.
    #[serde(default)]
    pub size_by: Option<String>,
}

/// One steering command, whichever it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Focus(Focus),
    Highlight(Highlight),
    Appearance(Appearance),
}

fn json_of<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("every control message is plain data")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{decode_frame, MessageType};

    fn one_frame(command: Command) -> (MessageType, serde_json::Value) {
        let frames = control_frames(&command);
        assert_eq!(frames.len(), 1, "a steering command is one JSON frame");
        let decoded = decode_frame(&frames[0]).expect("round trip");
        assert!(
            decoded.terminal,
            "a single-frame response ends on its frame"
        );
        (
            decoded.msg_type,
            serde_json::from_slice(&decoded.payload).expect("valid JSON payload"),
        )
    }

    #[test]
    fn focus_encodes_its_slots_under_its_own_message_type() {
        let (msg_type, json) = one_frame(Command::Focus(Focus::new(vec![3, 17, 4])));
        assert_eq!(msg_type, MessageType::Focus);
        assert_eq!(json["protocol_version"], PROTOCOL_VERSION);
        // Order is preserved, not sorted: an agent that asked to frame a path
        // gets the path it asked for.
        assert_eq!(json["slots"], serde_json::json!([3, 17, 4]));
    }

    #[test]
    fn an_empty_focus_is_a_legal_frame_the_whole_view_instruction() {
        // Not a degenerate case to reject: it is what follows a collapse.
        let (_, json) = one_frame(Command::Focus(Focus::new(Vec::new())));
        assert_eq!(json["slots"], serde_json::json!([]));
    }

    #[test]
    fn highlight_names_the_concept_in_the_wire_vocabulary() {
        // kebab-case on the wire, so a hand-written body and the generated
        // TypeScript agree without either side reading the other — the same
        // rule `Request` follows.
        let (msg_type, json) = one_frame(Command::Highlight(Highlight::new(
            vec![9],
            HighlightConcept::Selected,
        )));
        assert_eq!(msg_type, MessageType::Highlight);
        assert_eq!(json["concept"], "selected");

        let (_, json) = one_frame(Command::Highlight(Highlight::new(
            Vec::new(),
            HighlightConcept::Highlighted,
        )));
        assert_eq!(json["concept"], "highlighted");
        assert_eq!(json["slots"], serde_json::json!([]));
    }

    #[test]
    fn appearance_distinguishes_clearing_a_channel_from_leaving_it() {
        // `null` is "back to the structural encoding", and it has to survive
        // serialization as a present field: an omitted key would let a client
        // read "unchanged", which is a different instruction.
        let (msg_type, json) = one_frame(Command::Appearance(Appearance::new(
            Some("field".to_string()),
            None,
        )));
        assert_eq!(msg_type, MessageType::Appearance);
        assert_eq!(json["color_by"], "field");
        assert!(
            json.get("size_by").is_some(),
            "size_by must be present as null, not omitted: {json}"
        );
        assert!(json["size_by"].is_null());
    }
}
