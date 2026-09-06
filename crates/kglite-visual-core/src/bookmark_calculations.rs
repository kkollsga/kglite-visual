use std::collections::BTreeMap;

use crate::bookmark::{Bookmark, BOOKMARK_VERSION};
use crate::control::Appearance;
use crate::subset::{FrozenField, FrozenFields};
use crate::CoreError;

pub(crate) fn validate_version(bookmark: &Bookmark) -> Result<(), CoreError> {
    match bookmark.version {
        1 if bookmark.channels.is_none()
            && bookmark.calculations.is_none()
            && bookmark.derived.is_empty() =>
        {
            Ok(())
        }
        BOOKMARK_VERSION if bookmark.channels.is_some() && bookmark.calculations.is_some() => {
            Ok(())
        }
        1 => Err(refusal(
            "version 1 bookmarks cannot contain calculations or canonical channels",
        )),
        BOOKMARK_VERSION => Err(refusal(
            "version 2 bookmarks require canonical channels and calculation metadata",
        )),
        _ => Err(refusal("unsupported bookmark version")),
    }
}

pub(crate) fn appearance(bookmark: &Bookmark) -> Result<Appearance, CoreError> {
    match &bookmark.channels {
        Some(channels) => {
            for field in [&channels.color_field, &channels.size_field]
                .into_iter()
                .flatten()
            {
                field.validate()?;
            }
            let appearance =
                Appearance::from_fields(channels.color_field.clone(), channels.size_field.clone());
            if appearance.color_by != bookmark.color_by || appearance.size_by != bookmark.size_by {
                return Err(refusal(
                    "bookmark legacy appearance names contradict its canonical channels",
                ));
            }
            Ok(appearance)
        }
        None => Ok(Appearance::new(
            bookmark.color_by.clone(),
            bookmark.size_by.clone(),
        )),
    }
}

pub(crate) fn validate_values(bookmark: &Bookmark) -> Result<(), CoreError> {
    appearance(bookmark)?;
    let mut fields = FrozenFields::new();
    let mut member_ids = BTreeMap::new();
    for field in &bookmark.derived {
        let mut values = BTreeMap::new();
        for (member, value) in &field.values {
            let token =
                serde_json::to_string(member).map_err(|error| refusal(&error.to_string()))?;
            let next = member_ids.len() as u32;
            let id = *member_ids.entry(token).or_insert(next);
            if values.insert(id, value.clone()).is_some() {
                return Err(refusal("bookmark repeats a frozen field member"));
            }
        }
        if fields
            .insert(
                (field.calculation_id.clone(), field.column.clone()),
                FrozenField {
                    input_subset_revision: field.input_subset_revision.clone(),
                    values,
                },
            )
            .is_some()
        {
            return Err(refusal("bookmark repeats a derived field"));
        }
    }
    crate::calculations::validate_results(
        bookmark.calculations.as_deref().unwrap_or_default(),
        &fields,
    )
}

fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}
