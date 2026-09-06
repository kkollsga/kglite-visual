//! Preflight bounds viewer-owned copies and conservative encoded expansion.
//!
//! Engine disk/overflow reads may materialize a selected value before lending
//! it to this layer; this budget does not bound that engine allocation.
use std::time::Instant;

use kglite::api::Value;

use crate::output::{check_deadline, refusal, MAX_OUTPUT_PROPERTY_BYTES};
use crate::CoreError;

const MAX_DEPTH: usize = 32;
const MAX_VISITS: usize = 1_000_000;
pub(crate) const MAX_PROPERTIES_PER_ENTITY: usize = 4096;

pub(crate) struct PropertyBudget {
    owned: usize,
    visits: usize,
    pub encoded: usize,
    deadline: Instant,
}
impl PropertyBudget {
    pub fn new(deadline: Instant) -> Self {
        Self {
            owned: 0,
            visits: 0,
            encoded: 64 * 1024,
            deadline,
        }
    }

    pub fn entity(&mut self, node: bool) -> Result<(), CoreError> {
        self.charge(128, if node { 512 } else { 256 })
    }

    fn charge(&mut self, owned: usize, encoded: usize) -> Result<(), CoreError> {
        check_deadline(self.deadline)?;
        self.visits += 1;
        self.owned = self.owned.saturating_add(owned);
        self.encoded = self.encoded.saturating_add(encoded);
        if self.visits > MAX_VISITS || self.owned > MAX_OUTPUT_PROPERTY_BYTES {
            return Err(refusal(
                "output properties exceed the 8 MiB or one-million-value capture limit",
            ));
        }
        Ok(())
    }

    pub fn text(&mut self, text: &str) -> Result<(), CoreError> {
        // JSON may encode each source byte as six bytes; XML may escape each
        // resulting byte again. This also dominates CSV quoting and raw_string.
        self.charge(
            text.len().saturating_add(64),
            text.len().saturating_mul(36).saturating_add(64),
        )
    }

    pub fn value(&mut self, value: &Value, depth: usize) -> Result<(), CoreError> {
        if depth > MAX_DEPTH {
            return Err(refusal("output property nesting exceeds 32 levels"));
        }
        self.charge(64, 128)?;
        match value {
            Value::String(text) => self.text(text),
            Value::List(items) => {
                if items.len() > MAX_VISITS.saturating_sub(self.visits) {
                    return Err(refusal("output list exceeds remaining value budget"));
                }
                for item in items {
                    self.value(item, depth + 1)?;
                }
                Ok(())
            }
            Value::Map(items) => {
                if items.len() > MAX_VISITS.saturating_sub(self.visits) {
                    return Err(refusal("output map exceeds remaining value budget"));
                }
                for (key, item) in items {
                    self.text(key)?;
                    self.value(item, depth + 1)?;
                }
                Ok(())
            }
            Value::Float64(value) if !value.is_finite() => {
                Err(refusal("output contains a non-finite number"))
            }
            Value::Point { lat, lon } if !lat.is_finite() || !lon.is_finite() => {
                Err(refusal("output contains a non-finite coordinate"))
            }
            Value::NodeRef(_) | Value::Node(_) | Value::Relationship(_) | Value::Path(_) => Err(
                refusal("graph-valued properties cannot be exported as source attributes"),
            ),
            _ => Ok(()),
        }
    }

    pub fn clone_value(&mut self, value: &Value) -> Result<Value, CoreError> {
        self.value(value, 0)?;
        Ok(value.clone())
    }

    pub fn clone_text(&mut self, text: &str) -> Result<String, CoreError> {
        self.text(text)?;
        Ok(text.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{MAX_OUTPUT_BYTES, OUTPUT_TIMEOUT};

    #[test]
    fn owned_keys_and_nested_values_are_refused_before_clone() {
        let mut budget = PropertyBudget::new(Instant::now() + OUTPUT_TIMEOUT);
        assert!(budget
            .clone_text(&"k".repeat(MAX_OUTPUT_PROPERTY_BYTES + 1))
            .is_err());
        let mut nested = Value::Null;
        for _ in 0..34 {
            nested = Value::List(vec![nested]);
        }
        assert!(PropertyBudget::new(Instant::now() + OUTPUT_TIMEOUT)
            .clone_value(&nested)
            .is_err());
        assert!(PropertyBudget::new(Instant::now())
            .clone_value(&Value::Null)
            .is_err());
    }

    #[test]
    fn encoded_preflight_accounts_for_escape_expansion_before_writer() {
        let mut budget = PropertyBudget::new(Instant::now() + OUTPUT_TIMEOUT);
        budget
            .clone_value(&Value::String("\"<&".repeat(200_000)))
            .unwrap();
        assert!(budget.encoded > MAX_OUTPUT_BYTES);
    }
}
