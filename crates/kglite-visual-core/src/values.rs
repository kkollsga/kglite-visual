//! kglite `Value` at this crate's two outward boundaries.
//!
//! JSON conversion goes through kglite's own `kglite_value_to_json` and is not
//! re-implemented here: it is the canonical converter every binding shares, so
//! a cell in this app's results table renders the same way it does in the
//! wheel and in the MCP server. Re-implementing the dispatch would create a
//! second dialect of the engine's own data model — the shape `R8` names.
//!
//! The display half has no upstream equivalent, because a *label* is not a
//! serialization: a title of `null` must become the empty string, not the four
//! characters `null`, and a float must not arrive as `1.0999999999`.

use kglite::api::param::kglite_value_to_json;
use kglite::api::Value;

/// A `Value` as JSON, in kglite's canonical untagged shape.
pub fn value_to_json(value: &Value) -> serde_json::Value {
    kglite_value_to_json(value)
}

/// A `Value` as a short human label.
///
/// Null and the internal reference variants become the empty string: a label
/// layer that draws "Null" over a node is claiming the node is called Null.
pub fn value_to_display(value: &Value) -> String {
    match value {
        Value::Null | Value::NodeRef(_) => String::new(),
        Value::String(s) => s.clone(),
        Value::Int64(i) => i.to_string(),
        Value::UniqueId(u) => u.to_string(),
        // `{}` not `{:?}`: the debug form of an f64 prints the shortest
        // round-tripping representation, which is right for a baseline and
        // wrong for a caption.
        Value::Float64(f) => format!("{f}"),
        Value::Boolean(b) => b.to_string(),
        other => match kglite_value_to_json(other) {
            serde_json::Value::String(s) => s,
            json => json.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_null_title_is_an_empty_label_not_the_word_null() {
        assert_eq!(value_to_display(&Value::Null), "");
        assert_eq!(value_to_display(&Value::NodeRef(3)), "");
    }

    #[test]
    fn a_string_title_is_not_quoted() {
        // Through the JSON converter a string comes back with quotes, which is
        // correct for a cell and wrong for a label.
        assert_eq!(value_to_display(&Value::String("ada".into())), "ada");
        assert_eq!(value_to_json(&Value::String("ada".into())), "ada");
    }

    #[test]
    fn numbers_render_without_float_noise() {
        assert_eq!(value_to_display(&Value::Int64(-4)), "-4");
        assert_eq!(value_to_display(&Value::Float64(1.5)), "1.5");
        assert_eq!(value_to_display(&Value::Boolean(true)), "true");
    }
}
