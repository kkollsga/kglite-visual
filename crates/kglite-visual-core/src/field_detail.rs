//! Bounded source-field pages for expanded values; record cells remain previews.
use kglite::api::{NodeIndex, Value};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::records::{self, NodeHandle, RecordCell};
use crate::shared::RevisionStamp;
use crate::{CoreError, Session};

pub const MAX_FIELD_DETAIL_BYTES: usize = 256 * 1024;
pub const MAX_TEXT_PAGE_BYTES: usize = 32 * 1024;
pub const MAX_FIELD_PAGE_ITEMS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FieldPathSegment {
    Index { index: u32 },
    Key { key: String },
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FieldDetailRequest {
    pub handle: NodeHandle,
    /// A source property. Derived fields never reinterpret this name.
    pub field: String,
    #[serde(default)]
    pub path: Vec<FieldPathSegment>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FieldEntry {
    pub key: String,
    pub cell: RecordCell,
}
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FieldPage {
    Text {
        offset: u32,
        total_bytes: String,
        text: String,
        next_offset: Option<u32>,
    },
    List {
        offset: u32,
        total_items: u32,
        items: Vec<RecordCell>,
        next_offset: Option<u32>,
    },
    Map {
        offset: u32,
        total_items: u32,
        entries: Vec<FieldEntry>,
        next_offset: Option<u32>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct FieldDetailResponse {
    pub stamp: RevisionStamp,
    pub subset_revision: String,
    pub handle: NodeHandle,
    pub field: String,
    pub path: Vec<FieldPathSegment>,
    pub cell: RecordCell,
    pub page: Option<FieldPage>,
}
impl Session {
    pub fn field_detail(
        &self,
        request: &FieldDetailRequest,
    ) -> Result<FieldDetailResponse, CoreError> {
        validate(request)?;
        self.check_handles(std::slice::from_ref(&request.handle))?;
        let (stamp, subset_revision) = {
            let state = self.state_read();
            (
                state.stamp(self.generation()),
                state.subset_revision.to_string(),
            )
        };
        let _guard = self.graph().begin_read_pass();
        let (cell, page) = self.detail_value(request)?;
        let response = FieldDetailResponse {
            stamp,
            subset_revision,
            handle: request.handle.clone(),
            field: request.field.clone(),
            path: request.path.clone(),
            cell,
            page,
        };
        records::serialized_bytes(&response, MAX_FIELD_DETAIL_BYTES)?;
        Ok(response)
    }
    fn detail_value(
        &self,
        request: &FieldDetailRequest,
    ) -> Result<(RecordCell, Option<FieldPage>), CoreError> {
        let Some(node) = self
            .graph()
            .node_view(NodeIndex::new(request.handle.node_id as usize))
        else {
            return Ok((
                RecordCell::Unavailable {
                    reason: "node is absent from this source snapshot".into(),
                },
                None,
            ));
        };
        if request.field == "type" {
            return page_value(
                &Value::String(node.node_type_str(&self.graph().interner).into()),
                request,
            );
        }
        match node.get_field_ref(&request.field) {
            Some(value) => page_value(&value, request),
            None => Ok((
                records::read_cell(self.graph(), request.handle.node_id, &request.field),
                None,
            )),
        }
    }
}
fn validate(request: &FieldDetailRequest) -> Result<(), CoreError> {
    if request.field.is_empty() || request.field.len() > 256 {
        return Err(refusal("field name must contain 1–256 bytes"));
    }
    if request.path.len() > records::MAX_VALUE_DEPTH {
        return Err(refusal("field path exceeds depth 8"));
    }
    if request
        .path
        .iter()
        .any(|segment| matches!(segment,FieldPathSegment::Key{key} if key.len()>4096))
    {
        return Err(refusal("field path key exceeds 4096 bytes"));
    }
    if request.limit == Some(0) {
        return Err(refusal("field page limit must be positive"));
    }
    Ok(())
}
fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}
fn descend<'a>(mut value: &'a Value, path: &[FieldPathSegment]) -> Result<&'a Value, RecordCell> {
    for segment in path {
        value = match (value, segment) {
            (Value::List(items), FieldPathSegment::Index { index }) => {
                items.get(*index as usize).ok_or(RecordCell::Missing)?
            }
            (Value::Map(entries), FieldPathSegment::Key { key }) => {
                entries.get(key).ok_or(RecordCell::Missing)?
            }
            _ => {
                return Err(RecordCell::Unavailable {
                    reason: "field path does not match this value's container type".into(),
                })
            }
        };
    }
    Ok(value)
}
fn page_value(
    value: &Value,
    request: &FieldDetailRequest,
) -> Result<(RecordCell, Option<FieldPage>), CoreError> {
    let value = match descend(value, &request.path) {
        Ok(value) => value,
        Err(cell) => return Ok((cell, None)),
    };
    let limit = request
        .limit
        .unwrap_or(100)
        .min(MAX_FIELD_PAGE_ITEMS as u32) as usize;
    let page = match value {
        Value::String(text) => Some(text_page(text, request.offset)?),
        Value::List(items) => {
            let (values, next) = bounded_page(
                items
                    .iter()
                    .skip(request.offset as usize)
                    .map(|value| Ok(records::cell(value))),
                limit,
                request.offset,
                items.len(),
            )?;
            Some(FieldPage::List {
                offset: request.offset,
                total_items: item_count(items.len())?,
                items: values,
                next_offset: next,
            })
        }
        Value::Map(entries) => {
            let (values, next) = bounded_page(
                entries
                    .iter()
                    .skip(request.offset as usize)
                    .map(|(key, value)| {
                        if key.len() > 4096 {
                            return Err(refusal("map key exceeds 4096 byte addressable limit"));
                        }
                        Ok(FieldEntry {
                            key: key.to_string(),
                            cell: records::cell(value),
                        })
                    }),
                limit,
                request.offset,
                entries.len(),
            )?;
            Some(FieldPage::Map {
                offset: request.offset,
                total_items: item_count(entries.len())?,
                entries: values,
                next_offset: next,
            })
        }
        _ => None,
    };
    Ok((records::cell(value), page))
}
fn item_count(count: usize) -> Result<u32, CoreError> {
    u32::try_from(count).map_err(|_| refusal("value exceeds the addressable field page range"))
}
fn text_page(text: &str, offset: u32) -> Result<FieldPage, CoreError> {
    item_count(text.len())?;
    let start = offset as usize;
    if start > text.len() || !text.is_char_boundary(start) {
        return Err(refusal(
            "text page offset must be a UTF-8 byte boundary within the value",
        ));
    }
    let mut end = start.saturating_add(MAX_TEXT_PAGE_BYTES).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(FieldPage::Text {
        offset,
        total_bytes: text.len().to_string(),
        text: text[start..end].into(),
        next_offset: (end < text.len()).then_some(end as u32),
    })
}
fn bounded_page<T: Serialize>(
    items: impl Iterator<Item = Result<T, CoreError>>,
    limit: usize,
    offset: u32,
    total: usize,
) -> Result<(Vec<T>, Option<u32>), CoreError> {
    let mut page = Vec::new();
    let mut bytes = 0;
    for item in items.take(limit) {
        let item = item?;
        let remaining = (MAX_FIELD_DETAIL_BYTES - 64 * 1024usize).saturating_sub(bytes);
        let item_bytes = match records::serialized_bytes(&item, remaining) {
            Ok(bytes) => bytes + 1,
            Err(error) if page.is_empty() => return Err(error),
            Err(_) => break,
        };
        if item_bytes > remaining {
            break;
        }
        bytes += item_bytes;
        page.push(item);
    }
    let end = offset as usize + page.len();
    let next = (end < total && !page.is_empty())
        .then(|| item_count(end))
        .transpose()?;
    Ok((page, next))
}
