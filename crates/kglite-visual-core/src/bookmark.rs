//! Visualizer-owned bookmarks store exact membership, never an executable query.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::records::{NodeHandle, TypedValue};
use crate::request::LayoutKernel;
use crate::shared::{RevisionStamp, ViewReference};
use crate::source_identity::SourceFingerprint;
use crate::subset::SubsetFilter;

pub const BOOKMARK_VERSION: u32 = 1;
pub const MAX_BOOKMARK_BYTES: usize = 4 * 1024 * 1024;
pub use crate::bookmark_restore::validate_bookmark;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct BookmarkEligibility {
    pub storage: BookmarkStorage,
    pub reason: Option<String>,
    pub verification_required: bool,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct BookmarkCaptureOptions {
    #[serde(default)]
    #[ts(optional)]
    pub selected: Option<Vec<ViewReference>>,
    #[serde(default)]
    #[ts(optional)]
    pub focus: Option<BookmarkFocusRequest>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BookmarkFocusRequest {
    Fit,
    References { references: Vec<ViewReference> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum BookmarkFocus {
    Fit,
    References { references: Vec<BookmarkReference> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum BookmarkSource {
    Durable { source: SourceFingerprint },
    SessionOnly { generation: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum BookmarkMember {
    Key { node_type: String, key: TypedValue },
    Handle { handle: NodeHandle },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(deny_unknown_fields)]
pub struct BookmarkRelation {
    pub edge_id: u32,
    pub source_member: u32,
    pub target_member: u32,
    pub name: String,
    pub attributes_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum BookmarkReference {
    Member { member: u32 },
    Type { name: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(deny_unknown_fields)]
pub struct BookmarkPosition {
    pub reference: BookmarkReference,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(deny_unknown_fields)]
pub struct BookmarkLayout {
    pub kernel_requested: LayoutKernel,
    pub kernel_chosen: LayoutKernel,
    #[serde(deserialize_with = "required_option")]
    pub seed: Option<BookmarkReference>,
    pub positions: Vec<BookmarkPosition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(deny_unknown_fields)]
pub struct BookmarkDerivedField {
    pub calculation_id: String,
    pub column: String,
    pub input_subset_revision: String,
    pub values: Vec<(BookmarkMember, TypedValue)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub version: u32,
    pub source: BookmarkSource,
    pub members: Vec<BookmarkMember>,
    pub relations: Vec<BookmarkRelation>,
    pub predicates: Vec<SubsetFilter>,
    #[serde(deserialize_with = "required_option")]
    pub color_by: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub size_by: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub caption_by: Option<String>,
    #[serde(default)]
    pub presentation: crate::presentation::PresentationSettings,
    pub highlighted: Vec<BookmarkReference>,
    pub selected: Vec<BookmarkReference>,
    #[serde(deserialize_with = "required_option")]
    pub focus: Option<BookmarkFocus>,
    pub layout: BookmarkLayout,
    pub derived: Vec<BookmarkDerivedField>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct BookmarkCapture {
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) captured_selected: Vec<ViewReference>,
    pub bookmark: Bookmark,
    pub captured_stamp: RevisionStamp,
    pub content_revision: String,
    pub durability: BookmarkSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SavedViewMarker {
    #[serde(flatten)]
    pub bookmark: BookmarkName,
    pub content_revision: String,
    pub dirty: bool,
    pub selected: Vec<ViewReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum BookmarkStorage {
    Durable,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct BookmarkName {
    pub storage: BookmarkStorage,
    pub name: String,
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}
