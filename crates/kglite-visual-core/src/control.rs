//! Messages that steer a view without changing what is in it (plan D14).
//!
//! These payloads retain the legacy command vocabulary. Shared session actions
//! now persist appearance and explicit highlight identities in one revisioned
//! snapshot; focus is an ordered event and is not replayed on attachment.

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

/// Canonical field channels feed the shared server-computed appearance mapping.
/// Legacy names mirror source-property channels; null restores structural encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct Appearance {
    pub protocol_version: u32,
    pub color_by: Option<String>,
    pub size_by: Option<String>,
    pub color_field: Option<crate::subset::FieldRef>,
    pub size_field: Option<crate::subset::FieldRef>,
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
            color_field: color_by
                .clone()
                .map(|name| crate::subset::FieldRef::Property { name }),
            size_field: size_by
                .clone()
                .map(|name| crate::subset::FieldRef::Property { name }),
            color_by,
            size_by,
        }
    }
}

/// Encode the retained legacy command vocabulary. Live session updates use
/// the atomic `shared_frames` envelope.
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
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FocusRequest {
    /// Slots to frame. Empty frames the whole view.
    #[serde(default)]
    pub slots: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct HighlightRequest {
    /// Slots to mark. Empty clears the concept.
    #[serde(default)]
    pub slots: Vec<u32>,
    #[serde(default)]
    pub concept: HighlightConcept,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(try_from = "AppearanceInput")]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct AppearanceRequest {
    #[ts(optional = nullable)]
    pub color_by: Option<String>,
    #[ts(optional = nullable)]
    pub size_by: Option<String>,
    #[ts(optional)]
    pub color_field: Option<Option<crate::subset::FieldRef>>,
    #[ts(optional)]
    pub size_field: Option<Option<crate::subset::FieldRef>>,
}

#[derive(Deserialize)]
struct AppearanceInput {
    #[serde(default, deserialize_with = "present_option")]
    color_by: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_option")]
    size_by: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_option")]
    color_field: Option<Option<crate::subset::FieldRef>>,
    #[serde(default, deserialize_with = "present_option")]
    size_field: Option<Option<crate::subset::FieldRef>>,
}
fn present_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}
impl TryFrom<AppearanceInput> for AppearanceRequest {
    type Error = String;
    fn try_from(input: AppearanceInput) -> Result<Self, Self::Error> {
        for (legacy, canonical) in [
            (&input.color_by, &input.color_field),
            (&input.size_by, &input.size_field),
        ] {
            if let (Some(legacy), Some(canonical)) = (legacy, canonical) {
                let property = legacy
                    .clone()
                    .map(|name| crate::subset::FieldRef::Property { name });
                if &property != canonical {
                    return Err(
                        "legacy and canonical appearance fields contradict each other".into(),
                    );
                }
            }
        }
        Ok(Self {
            color_by: input.color_by.flatten(),
            size_by: input.size_by.flatten(),
            color_field: input.color_field,
            size_field: input.size_field,
        })
    }
}
impl AppearanceRequest {
    pub(crate) fn resolve(&self) -> Result<Appearance, crate::CoreError> {
        let channel = |legacy: &Option<String>,
                       canonical: &Option<Option<crate::subset::FieldRef>>|
         -> Result<_, crate::CoreError> {
            let property = legacy
                .clone()
                .map(|name| crate::subset::FieldRef::Property { name });
            if legacy.is_some() && canonical.as_ref().is_some_and(|field| field != &property) {
                return Err(crate::CoreError::Request(
                    "legacy and canonical appearance fields contradict each other".into(),
                ));
            }
            let field = canonical.clone().unwrap_or(property);
            if let Some(field) = &field {
                field.validate()?;
            }
            Ok(field)
        };
        Ok(Appearance::from_fields(
            channel(&self.color_by, &self.color_field)?,
            channel(&self.size_by, &self.size_field)?,
        ))
    }
}
impl Appearance {
    pub(crate) fn from_fields(
        color_field: Option<crate::subset::FieldRef>,
        size_field: Option<crate::subset::FieldRef>,
    ) -> Self {
        let source_name = |field: &Option<crate::subset::FieldRef>| match field {
            Some(crate::subset::FieldRef::Property { name }) => Some(name.clone()),
            _ => None,
        };
        Self {
            protocol_version: PROTOCOL_VERSION,
            color_by: source_name(&color_field),
            size_by: source_name(&size_field),
            color_field,
            size_field,
        }
    }
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
