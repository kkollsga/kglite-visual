//! Parse-only validation: what is wrong with this query, without running it.
//!
//! **kglite offers a real parse-only entry point, so this is not an
//! approximation.** `kglite::api::cypher::parse_cypher` takes a `&str` and
//! returns a `CypherQuery` or a `KgError` — it has no `&DirGraph` argument, so
//! it structurally cannot touch data, and it is the *cached* parser
//! `session::execute` itself uses, so a repeated statement costs a hash lookup
//! and an AST clone. The `EXPLAIN` fallback this module was scoped against was
//! not needed and is not used; `EXPLAIN` still plans, and planning is more work
//! than answering "does this parse".
//!
//! **`KgError` already carries the caret**, as `line` and `col` on
//! `CypherSyntax`, 1-indexed, read here through the public `position()`
//! accessor rather than scraped out of the message text. A parser error with no
//! position ("expected end of input") is reported against the whole document
//! instead of being dropped or pinned to a guess.
//!
//! **What this can see, and what only running can.** Three checks, and the
//! boundary is deliberate:
//!
//! * the parse, which is exact;
//! * the same unknown-label / unknown-relationship / absent-property
//!   advisories the engine attaches to a real run
//!   (`collect_unknown_pattern_warnings`), as warnings — a zero-row existence
//!   check is legal Cypher, so these are never errors;
//! * whether the statement mutates, which this viewer will refuse: `execute_read`
//!   answers a mutation with `KgError::Argument` and nothing else, so saying it
//!   before the user presses Run is telling them what the engine has already
//!   decided.
//!
//! It does **not** run kglite's schema validation (`validate_schema`) or its
//! planner. Those need the parameter bindings a validation request does not
//! carry, and a `MATCH (n:$label)` would be reported as an unknown type — a
//! confident error about a correct query, which is worse than a missing one.
//! The run is still the authority; this is the fast, cheap half of it.

use kglite::api::cypher::{collect_unknown_pattern_warnings, is_mutation_query, parse_cypher};
use kglite::api::DirGraph;
use serde::Serialize;
use ts_rs::TS;

use crate::protocol::PROTOCOL_VERSION;

/// One finding, positioned where the engine positioned it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct Diagnostic {
    /// `"error"` — the query cannot run — or `"warning"` — it can, and may not
    /// answer the question that was asked.
    pub severity: DiagnosticSeverity,
    /// kglite's own sentence wherever kglite produced one. It carries the
    /// offending name and a "did you mean?" hint, which is the whole of what
    /// the user can act on.
    pub message: String,
    /// 1-indexed line, or `null` when the finding is about the whole query.
    pub line: Option<u32>,
    /// 1-indexed column, `null` under the same rule as `line`.
    pub col: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

/// What `POST /api/validate` answers.
///
/// An empty list means "nothing found", never "not checked": the endpoint has
/// one failure mode — the request was malformed — and that is an HTTP status,
/// not an empty success (`R10` corollary, green and not-attempted must not
/// render identically).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ValidateResponse {
    pub protocol_version: u32,
    pub diagnostics: Vec<Diagnostic>,
}

/// Read-only wording, in one place because the handler doc quotes it.
const MUTATION_REFUSAL: &str =
    "this viewer runs queries read-only — the engine will refuse a statement that writes";

/// Parse `query` and report what is wrong with it. Nothing is executed.
pub fn validate_query(graph: &DirGraph, query: &str) -> ValidateResponse {
    let mut diagnostics = Vec::new();
    // An empty box is not a mistake, and underlining it would put a red squiggle
    // under the editor the moment it opens.
    if query.trim().is_empty() {
        return ValidateResponse {
            protocol_version: PROTOCOL_VERSION,
            diagnostics,
        };
    }

    match parse_cypher(query) {
        Err(err) => {
            let (line, col) = match err.position() {
                Some((line, col)) => (Some(line as u32), Some(col as u32)),
                None => (None, None),
            };
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: err.to_string(),
                line,
                col,
            });
        }
        Ok(parsed) => {
            if is_mutation_query(&parsed) {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: MUTATION_REFUSAL.to_string(),
                    line: None,
                    col: None,
                });
            }
            // Positionless by construction: kglite's advisory channel is a list
            // of sentences, each naming the label or property it is about, and
            // inventing a position by searching the text for that name would
            // underline the wrong occurrence on a query that mentions it twice.
            for message in collect_unknown_pattern_warnings(&parsed, graph) {
                diagnostics.push(Diagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message,
                    line: None,
                    col: None,
                });
            }
        }
    }

    ValidateResponse {
        protocol_version: PROTOCOL_VERSION,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kglite::api::session::{execute_mut, execute_read, ExecuteOptions};
    use std::collections::HashMap;

    /// Two Person nodes and one relationship — enough schema for the advisory
    /// channel to have something to be wrong about.
    fn graph() -> DirGraph {
        let mut graph = DirGraph::new();
        let empty = HashMap::new();
        execute_mut(
            &mut graph,
            "CREATE (a:Person {title: 'a'})-[:KNOWS]->(b:Person {title: 'b'})\n",
            &ExecuteOptions::eager(&empty),
        )
        .expect("fixture builds");
        graph
    }

    /// Node count, read the way anything else would read it.
    fn nodes(graph: &DirGraph) -> i64 {
        let empty = HashMap::new();
        let result = execute_read(
            graph,
            "MATCH (n) RETURN count(n) AS n",
            &ExecuteOptions::eager(&empty),
        )
        .expect("count runs");
        match result.result.rows.first().and_then(|row| row.first()) {
            Some(kglite::api::Value::Int64(count)) => *count,
            other => panic!("unexpected count cell: {other:?}"),
        }
    }

    #[test]
    fn a_syntax_error_carries_the_engines_caret() {
        let response = validate_query(&graph(), "MATCH (p:Person RETURN p");
        assert_eq!(response.diagnostics.len(), 1, "{:?}", response.diagnostics);
        let found = &response.diagnostics[0];
        assert_eq!(found.severity, DiagnosticSeverity::Error);
        // A caret, not a whole-document underline: the position is what makes
        // this worth showing in an editor at all.
        assert!(found.line.is_some(), "no line in {found:?}");
        assert!(found.col.is_some(), "no column in {found:?}");
    }

    #[test]
    fn a_valid_query_reports_nothing() {
        let response = validate_query(&graph(), "MATCH (p:Person) RETURN p.title LIMIT 3");
        assert!(
            response.diagnostics.is_empty(),
            "{:?}",
            response.diagnostics
        );
    }

    #[test]
    fn an_unknown_label_is_a_warning_rather_than_an_error() {
        let response = validate_query(&graph(), "MATCH (p:Persn) RETURN p");
        // Zero-row existence checks are legal Cypher, so this must never stop
        // the query from being runnable.
        assert_eq!(response.diagnostics.len(), 1, "{:?}", response.diagnostics);
        assert_eq!(
            response.diagnostics[0].severity,
            DiagnosticSeverity::Warning
        );
        assert!(
            response.diagnostics[0].message.contains("Persn"),
            "the engine's own wording was replaced: {:?}",
            response.diagnostics[0].message
        );
    }

    #[test]
    fn a_write_is_refused_before_it_runs_rather_than_after() {
        let response = validate_query(&graph(), "CREATE (n:Person {title: 'x'})");
        assert_eq!(response.diagnostics.len(), 1, "{:?}", response.diagnostics);
        assert_eq!(response.diagnostics[0].severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn an_empty_query_is_not_a_mistake() {
        assert!(validate_query(&graph(), "   \n  ").diagnostics.is_empty());
    }

    #[test]
    fn explain_parses_like_any_other_statement() {
        let response = validate_query(&graph(), "EXPLAIN MATCH (p:Person) RETURN p");
        assert!(
            response.diagnostics.is_empty(),
            "{:?}",
            response.diagnostics
        );
    }

    /// The guarantee the whole endpoint rests on: validating does not execute.
    ///
    /// Validating a `CREATE` against a graph and finding the same node count
    /// afterwards is the observable half of "parse-only" — the half a reader
    /// of `parse_cypher`'s signature can only infer.
    #[test]
    fn validation_never_touches_the_graph() {
        let graph = graph();
        let before = nodes(&graph);
        validate_query(&graph, "CREATE (n:Person {title: 'ghost'})");
        validate_query(&graph, "MATCH (a)-[:KNOWS]->(b) DELETE a");
        assert_eq!(nodes(&graph), before);
    }
}
