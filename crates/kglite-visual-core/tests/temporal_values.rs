//! kglite 0.20.0 values reach the records layer: zoned datetimes, times of day
//! and sub-second durations. Before the floor move `typed_value` matched
//! `Value::Duration` without `nanos` and had no arm for the three new variants,
//! so the crate did not compile; these pin what each one renders as.

use kglite::api::session::{execute_read, ExecuteOptions};
use kglite::api::{DirGraph, Value};
use kglite_visual_core::records::{cell, typed_value, RecordCell, TypedValue};

fn evaluate(expression: &str) -> Value {
    let graph = DirGraph::new();
    let empty = Default::default();
    let result = execute_read(
        &graph,
        &format!("RETURN {expression} AS v"),
        &ExecuteOptions::eager(&empty),
    )
    .expect("query runs");
    result.result.rows[0][0].clone()
}

fn typed(expression: &str) -> TypedValue {
    typed_value(&evaluate(expression), 0).expect("converts")
}

#[test]
fn a_zoned_datetime_keeps_its_offset_in_iso_text() {
    assert_eq!(
        typed("datetime('2024-03-15T10:30:00+02:00')"),
        TypedValue::Timestamp("2024-03-15T10:30+02:00".into())
    );
    assert_eq!(
        typed("datetime('2024-03-15T10:30:00Z')"),
        TypedValue::Timestamp("2024-03-15T10:30Z".into())
    );
}

#[test]
fn a_time_of_day_is_text_not_a_timestamp() {
    assert_eq!(
        typed("localtime('10:00:00.5')"),
        TypedValue::String("10:00:00.500".into())
    );
    assert_eq!(
        typed("time('10:00:00+01:00')"),
        TypedValue::String("10:00+01:00".into())
    );
}

#[test]
fn a_sub_second_duration_keeps_its_fraction() {
    let seconds = |expression: &str| match typed(expression) {
        TypedValue::Duration { seconds, .. } => seconds,
        other => panic!("duration expected, got {other:?}"),
    };
    assert_eq!(seconds("duration({seconds: 90})"), "90");
    assert_eq!(seconds("duration({nanoseconds: 1})"), "0.000000001");
    assert_eq!(seconds("duration({seconds: 1.5})"), "1.5");
    assert_eq!(seconds("duration({seconds: -0.5})"), "-0.5");
    assert_eq!(
        seconds("duration({seconds: -1, nanoseconds: -500000000})"),
        "-1.5"
    );
}

#[test]
fn the_new_values_are_record_cells_not_omissions() {
    for expression in [
        "datetime('2024-03-15T10:30:00+02:00')",
        "localtime('10:00')",
        "time('10:00Z')",
        "duration({milliseconds: 1})",
    ] {
        assert!(
            matches!(cell(&evaluate(expression)), RecordCell::Value { .. }),
            "{expression}"
        );
    }
}
