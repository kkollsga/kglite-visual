//! Adapter regressions for frozen calculation fields and canonical appearance.
use super::*;
use serde_json::{json, Value};

async fn fixture() -> (
    AppState,
    ViewControl,
    Vec<kglite_visual_core::records::NodeHandle>,
) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../kglite-visual-core/tests/fixtures/viewer-identity.kgl");
    let graph =
        kglite_visual_core::load_graph(kglite_visual_core::GraphSource::Path(&path)).unwrap();
    let state = AppState::new(
        Arc::new(kglite_visual_core::Session::open(graph, "calculations")),
        "calculations",
    );
    state
        .execute(SharedRequest::new(Request::Cypher(CypherRequest {
            query: "MATCH (n) RETURN n".into(),
            params: Default::default(),
            limit: None,
            as_graph: true,
        })))
        .await
        .unwrap();
    let handles = state.session.snapshot_shared().meta.subset.visible_nodes;
    let control = ViewControl::new(state.clone());
    (state, control, handles)
}

fn value(result: CallToolResult) -> Value {
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let encoded = serde_json::to_value(result).unwrap();
    serde_json::from_str(encoded["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn calculations_are_discoverable_and_records_keep_source_and_derived_namespaces_distinct() {
    let (state, control, handles) = fixture().await;
    let mut events = state.bus.subscribe();
    let before = state.session.shared_stamp();
    let calculated = value(
        control
            .calculate(Parameters(
                serde_json::from_value(json!({
                    "kind":"degree", "expected":before, "request_id":"derived-records",
                }))
                .unwrap(),
            ))
            .await
            .unwrap(),
    );
    events.try_recv().unwrap();
    assert!(events.try_recv().is_err());
    assert_eq!(calculated["request_id"], "derived-records");
    let read = value(control.view_state().await.unwrap());
    assert_eq!(read["calculations"], calculated["state"]["calculations"]);
    let field = read["calculations"][0]["fields"][2]["field"].clone();
    let table = value(
        control
            .records(Parameters(
                serde_json::from_value(json!({
                    "handles":handles, "fields":["@derived:degree:total"], "field_refs":[field],
                }))
                .unwrap(),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(
        table["columns"][0]["field"],
        json!({"kind":"property","name":"@derived:degree:total"})
    );
    assert_eq!(table["columns"][1]["field"], field);
    for row in table["rows"].as_array().unwrap() {
        assert_eq!(row["cells"][0]["state"], "missing");
        assert_eq!(
            row["cells"][1],
            json!({"state":"value","value":{"type":"int64","value":"0"}})
        );
    }
    let no_legacy_fields = value(
        control
            .records(Parameters(
                serde_json::from_value(json!({
                    "handles":handles,"field_refs":[field],
                }))
                .unwrap(),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(no_legacy_fields["columns"].as_array().unwrap().len(), 1);
    assert!(events.try_recv().is_err());
    assert_eq!(
        state.session.shared_stamp().revision,
        calculated["state"]["stamp"]["revision"].as_str().unwrap()
    );
    let stale = control
        .calculate(Parameters(
            serde_json::from_value(json!({"kind":"degree","expected":before})).unwrap(),
        ))
        .await
        .unwrap();
    assert!(stale.is_error.unwrap_or(false));
    assert!(events.try_recv().is_err());
    assert_eq!(state.session.snapshot_shared().meta.calculations.len(), 1);
}

#[tokio::test]
async fn canonical_appearance_preserves_null_presence_and_applies_with_presentation_atomically() {
    let (state, control, _) = fixture().await;
    let result = value(
        control
            .calculate(Parameters(
                serde_json::from_value(json!({"kind":"degree"})).unwrap(),
            ))
            .await
            .unwrap(),
    );
    let field = result["state"]["calculations"][0]["fields"][2]["field"].clone();
    let mut events = state.bus.subscribe();
    let styled = value(control.set_appearance(Parameters(serde_json::from_value(json!({
        "size_field":field,"presentation":{"edge_opacity":0.3},"expected":state.session.shared_stamp(),
    })).unwrap())).await.unwrap());
    events.try_recv().unwrap();
    assert!(events.try_recv().is_err());
    assert_eq!(styled["state"]["appearance"]["size_field"], field);
    assert!(styled["state"]["appearance"]["size_by"].is_null());
    let before = state.session.snapshot_shared();
    for conflict in [
        json!({"size_by":null,"size_field":field}),
        json!({"size_by":"score","size_field":field}),
        json!({"size_by":"score","size_field":null}),
    ] {
        assert!(control
            .set_appearance(Parameters(serde_json::from_value(conflict).unwrap()))
            .await
            .is_err());
        assert_eq!(state.session.snapshot_shared(), before);
        assert!(events.try_recv().is_err());
    }
    let cleared = value(
        control
            .set_appearance(Parameters(
                serde_json::from_value(json!({"size_field":null})).unwrap(),
            ))
            .await
            .unwrap(),
    );
    assert!(cleared["size_field"].is_null());
    assert!(cleared["size_by"].is_null());
    events.try_recv().unwrap();
    assert!(events.try_recv().is_err());
}
