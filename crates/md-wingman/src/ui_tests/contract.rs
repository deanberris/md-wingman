//! The app's half of the contract in `service/fixtures`: the structured
//! replies the service's tests check against Claude's schemas must parse
//! here, and every item must show up against its passage.

use gpui_kit::TestAppContext;
use serde_json::Value;
use wingman_client::{Event, TaskKind};

use super::*;
use crate::suggestions::{CopyEditReply, FactCheckReply, ReferencesReply};

fn fixture(kind: TaskKind) -> (String, Value) {
    let text = match kind {
        TaskKind::CopyEdit => include_str!("../../../../service/fixtures/copy_edit.json"),
        TaskKind::FactCheck => include_str!("../../../../service/fixtures/fact_check.json"),
        TaskKind::FindReferences => {
            include_str!("../../../../service/fixtures/find_references.json")
        }
    };
    let v: Value = serde_json::from_str(text).expect("fixture is JSON");
    (
        v["passage"].as_str().unwrap().to_string(),
        v["data"].clone(),
    )
}

#[test]
fn fixtures_parse_as_replies() {
    let (_, data) = fixture(TaskKind::CopyEdit);
    let r: CopyEditReply = serde_json::from_value(data).unwrap();
    assert_eq!(r.edits.len(), 3);
    let (_, data) = fixture(TaskKind::FactCheck);
    let r: FactCheckReply = serde_json::from_value(data).unwrap();
    assert_eq!(r.claims.len(), 2);
    let (_, data) = fixture(TaskKind::FindReferences);
    let r: ReferencesReply = serde_json::from_value(data).unwrap();
    assert_eq!(r.references.len(), 2);
}

/// Runs `kind` on the fixture's passage and answers with its reply.
fn answer_with_fixture(cx: &mut TestAppContext, kind: TaskKind) -> (Harness, Vec<String>) {
    let (passage, data) = fixture(kind);
    let doc: &'static str = Box::leak(format!("# A post\n\n{passage}").into_boxed_str());
    let h = open(
        cx,
        Setup {
            text: doc,
            ..Default::default()
        },
    );
    h.cursor_at(cx, &passage[..10]);
    h.click(
        cx,
        match kind {
            TaskKind::CopyEdit => "run-copy-edit",
            TaskKind::FactCheck => "run-fact-check",
            TaskKind::FindReferences => "run-references",
        },
    );
    assert_eq!(h.assistant.requests(), vec![(kind, passage.clone())]);
    let summary = data["summary"].as_str().unwrap().to_string();
    h.assistant.send(
        0,
        [Event::Done {
            text: summary.clone(),
            data: Some(data),
        }],
    );
    cx.run_until_parked();
    assert_eq!(
        h.read(cx, |ws, _| ws.run_cards()),
        vec![(kind, format!("done: {summary}"))]
    );
    let shown = h.open_findings(cx);
    (h, shown)
}

#[gpui_kit::test]
fn the_copy_edit_fixture_shows_every_edit(cx: &mut TestAppContext) {
    let (_h, shown) = answer_with_fixture(cx, TaskKind::CopyEdit);
    assert_eq!(shown, vec!["Their are", "teh", "it were"]);
}

#[gpui_kit::test]
fn the_fact_check_fixture_shows_every_claim(cx: &mut TestAppContext) {
    let (_h, shown) = answer_with_fixture(cx, TaskKind::FactCheck);
    assert_eq!(
        shown,
        vec![
            "The Eiffel Tower opened in 1899.",
            "It is about 330 metres tall."
        ]
    );
}

#[gpui_kit::test]
fn the_references_fixture_shows_every_reference(cx: &mut TestAppContext) {
    let (_h, shown) = answer_with_fixture(cx, TaskKind::FindReferences);
    assert_eq!(
        shown,
        vec![
            "Coffee improves short-term memory.",
            "Most people drink it in the morning."
        ]
    );
}
