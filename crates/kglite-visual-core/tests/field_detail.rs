use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::{DirGraph, Value};
use kglite_visual_core::field_detail::{
    FieldDetailRequest, FieldPage, FieldPathSegment, MAX_FIELD_DETAIL_BYTES, MAX_TEXT_PAGE_BYTES,
};
use kglite_visual_core::records::{NodeHandle, RecordCell, TypedValue};
use kglite_visual_core::Session;
use std::sync::Arc;

fn session(text: String) -> Session {
    let mut graph = DirGraph::new();
    let params = [("text".into(), Value::String(text))].into_iter().collect();
    execute_mut(&mut graph,"CREATE (:P {id:null,title:'record',text:$text,items:range(0,299),nested:{note:$text,large:9007199254740993}})",&ExecuteOptions::eager(&params)).unwrap();
    Session::open(Arc::new(graph), "detail")
}
fn request(session: &Session, field: &str) -> FieldDetailRequest {
    FieldDetailRequest {
        handle: session.node_handle(0),
        field: field.into(),
        path: vec![],
        offset: 0,
        limit: None,
    }
}
#[test]
fn text_pages_reconstruct_unicode_without_splitting_codepoints_or_mutating_view() {
    let original = "A🦀".repeat(25_000);
    let session = session(original.clone());
    let before = session.shared_stamp();
    let mut request = request(&session, "text");
    let mut copied = String::new();
    let mut pages = 0;
    loop {
        let detail = session.field_detail(&request).unwrap();
        assert_eq!(detail.stamp, before);
        assert!(matches!(detail.cell, RecordCell::Truncated { .. }));
        assert!(serde_json::to_vec(&detail).unwrap().len() <= MAX_FIELD_DETAIL_BYTES);
        let Some(FieldPage::Text {
            offset,
            total_bytes,
            text,
            next_offset,
        }) = detail.page
        else {
            panic!("text page")
        };
        assert_eq!(offset, request.offset);
        assert_eq!(total_bytes, original.len().to_string());
        assert!(text.len() <= MAX_TEXT_PAGE_BYTES);
        copied.push_str(&text);
        pages += 1;
        match next_offset {
            Some(next) => request.offset = next,
            None => break,
        }
    }
    assert!(pages > 1);
    assert_eq!(copied, original);
    assert_eq!(session.shared_stamp(), before);
}
#[test]
fn escaped_text_page_stays_inside_serialized_byte_limit() {
    let session = session("\0".repeat(100_000));
    let detail = session.field_detail(&request(&session, "text")).unwrap();
    assert!(serde_json::to_vec(&detail).unwrap().len() <= MAX_FIELD_DETAIL_BYTES);
    assert!(matches!(
        detail.page,
        Some(FieldPage::Text {
            next_offset: Some(_),
            ..
        })
    ));
}
#[test]
fn collection_pages_and_nested_paths_keep_exact_integer_values() {
    let session = session("long".repeat(5000));
    let mut page = request(&session, "items");
    page.limit = Some(999);
    let first = session.field_detail(&page).unwrap();
    let Some(FieldPage::List {
        items,
        next_offset,
        total_items,
        ..
    }) = first.page
    else {
        panic!("list")
    };
    assert_eq!(items.len(), 128);
    assert_eq!(total_items, 300);
    assert_eq!(next_offset, Some(128));
    page.offset = 128;
    assert!(matches!(
        session.field_detail(&page).unwrap().page,
        Some(FieldPage::List {
            next_offset: Some(256),
            ..
        })
    ));
    let mut nested = request(&session, "nested");
    nested.path = vec![FieldPathSegment::Key {
        key: "large".into(),
    }];
    assert_eq!(
        session.field_detail(&nested).unwrap().cell,
        RecordCell::Value {
            value: TypedValue::Int64("9007199254740993".into())
        }
    );
    nested.path = vec![FieldPathSegment::Key { key: "note".into() }];
    assert!(matches!(
        session.field_detail(&nested).unwrap().page,
        Some(FieldPage::Text { .. })
    ));
    nested.path = vec![FieldPathSegment::Key {
        key: "absent".into(),
    }];
    assert_eq!(
        session.field_detail(&nested).unwrap().cell,
        RecordCell::Missing
    );
}
#[test]
fn null_missing_unavailable_and_invalid_paths_are_distinct() {
    let session = session("🦀".repeat(100));
    assert_eq!(
        session.field_detail(&request(&session, "id")).unwrap().cell,
        RecordCell::Null
    );
    assert_eq!(
        session
            .field_detail(&request(&session, "absent"))
            .unwrap()
            .cell,
        RecordCell::Missing
    );
    let mut invalid = request(&session, "text");
    invalid.path = vec![FieldPathSegment::Index { index: 0 }];
    assert!(matches!(
        session.field_detail(&invalid).unwrap().cell,
        RecordCell::Unavailable { .. }
    ));
    invalid.path.clear();
    invalid.offset = 1;
    assert!(session.field_detail(&invalid).is_err());
    invalid.offset = 0;
    invalid.path = vec![FieldPathSegment::Index { index: 0 }; 9];
    assert!(session.field_detail(&invalid).is_err());
    invalid.path.clear();
    invalid.handle = NodeHandle {
        generation: session.generation().into(),
        node_id: 999,
    };
    assert!(matches!(
        session.field_detail(&invalid).unwrap().cell,
        RecordCell::Unavailable { .. }
    ));
    invalid.handle.generation = "foreign".into();
    assert!(session.field_detail(&invalid).is_err());
}

#[test]
fn an_unaddressable_map_key_is_refused_before_page_cloning() {
    let mut graph = DirGraph::new();
    let key = "key".repeat(1_000_000);
    let map = Value::Map([(key.as_str(), Value::Int64(1))].into_iter().collect());
    let params = [("map".into(), map)].into_iter().collect();
    execute_mut(
        &mut graph,
        "CREATE (:P {id:1,nested:$map})",
        &ExecuteOptions::eager(&params),
    )
    .unwrap();
    let session = Session::open(Arc::new(graph), "huge-key");
    let before = session.shared_stamp();
    let error = session
        .field_detail(&request(&session, "nested"))
        .unwrap_err();
    assert!(error.to_string().contains("addressable"), "{error}");
    assert_eq!(session.shared_stamp(), before);
}
