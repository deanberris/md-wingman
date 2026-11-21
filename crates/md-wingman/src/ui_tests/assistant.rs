//! Running the assistant from the sidebar and shortcuts: copy-edit,
//! fact-check and references, and what the cards show.

use gpui_kit::TestAppContext;
use wingman_client::{Event, TaskKind};

use super::*;

const DOC: &str =
    "Their are three towers in teh city.\n\nThe second paragraph is about something else entirely.";

#[gpui_kit::test]
fn copy_edit_from_the_sidebar_suggests_and_accepts(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "three");
    h.click(cx, "run-copy-edit");

    // Only the paragraph under the cursor is sent, with the document as context.
    assert_eq!(
        h.assistant.requests(),
        vec![(
            TaskKind::CopyEdit,
            "Their are three towers in teh city.".to_string()
        )]
    );
    assert_eq!(h.assistant.context(0).as_deref(), Some(DOC));

    h.assistant.send(
        0,
        copy_edit_reply(&[("Their are", "There are"), ("teh", "the")]),
    );
    cx.run_until_parked();
    assert_eq!(h.open_findings(cx), vec!["Their are", "teh"]);

    // Accept one: the text changes and that suggestion goes.
    let teh = h.finding_id(cx, "teh");
    h.click(cx, finding_button("accept", "run", teh));
    assert!(h
        .text(cx)
        .starts_with("Their are three towers in the city."));
    assert_eq!(h.open_findings(cx), vec!["Their are"]);

    // Dismiss the other: the text stays as it is.
    let their = h.finding_id(cx, "Their are");
    h.click(cx, finding_button("dismiss", "run", their));
    assert!(h.open_findings(cx).is_empty());
    assert!(h.text(cx).starts_with("Their are three"));
}

#[gpui_kit::test]
fn copy_edit_shortcut_sends_the_selection_and_accepts_all_as_one_undo(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.select(cx, "Their are three towers in teh city.");
    h.press(cx, &shortcut("e"));
    assert_eq!(
        h.assistant.requests(),
        vec![(
            TaskKind::CopyEdit,
            "Their are three towers in teh city.".to_string()
        )]
    );
    h.assistant.send(
        0,
        copy_edit_reply(&[("Their are", "There are"), ("teh", "the")]),
    );
    cx.run_until_parked();
    let run = h.read(cx, |ws, _| ws.run_ids()[0]);
    assert_eq!(
        h.read(cx, |ws, _| ws.run_cards()),
        vec![(TaskKind::CopyEdit, "done: 2 suggested edits".to_string())]
    );

    h.click(cx, ("accept-all", run));
    assert!(h
        .text(cx)
        .starts_with("There are three towers in the city."));
    assert!(h.open_findings(cx).is_empty());

    // Both edits went in as one change: one undo takes both back, and the
    // suggestions come back with their text.
    h.press(cx, UNDO);
    assert_eq!(h.text(cx), DOC);
    assert_eq!(h.open_findings(cx), vec!["Their are", "teh"]);
}

#[gpui_kit::test]
fn a_run_streams_then_shows_its_summary(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "second");
    h.click(cx, "run-fact-check");
    assert_eq!(
        h.read(cx, |ws, _| ws.run_cards()),
        vec![(TaskKind::FactCheck, "running: Thinking…".to_string())]
    );
    h.assistant.send(
        0,
        [Event::Status {
            message: "Using WebSearch".into(),
        }],
    );
    cx.run_until_parked();
    assert_eq!(
        h.read(cx, |ws, _| ws.run_cards()),
        vec![(TaskKind::FactCheck, "running: Using WebSearch…".to_string())]
    );
    // A cancel button while it runs.
    let run = h.read(cx, |ws, _| ws.run_ids()[0]);
    assert!(h.exists(cx, ("cancel", run)));

    h.assistant.send(0, fact_check_reply(&[]));
    cx.run_until_parked();
    assert_eq!(
        h.read(cx, |ws, _| ws.run_cards()),
        vec![(TaskKind::FactCheck, "done: Checked the claims".to_string())]
    );
    assert!(h.exists(cx, ("close", run)));
}

#[gpui_kit::test]
fn cancelling_a_run_says_so(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "three");
    h.click(cx, "run-references");
    let run = h.read(cx, |ws, _| ws.run_ids()[0]);
    h.click(cx, ("cancel", run));
    assert!(h.assistant.was_cancelled(0));
    assert_eq!(
        h.read(cx, |ws, _| ws.run_cards()),
        vec![(TaskKind::FindReferences, "failed: Cancelled".to_string())]
    );
}

/// What the app shows when the service fails a task, such as when Claude
/// gives no structured reply: the service's message, on the card.
#[gpui_kit::test]
fn a_service_error_shows_on_the_card(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "three");
    h.click(cx, "run-copy-edit");
    h.assistant.send(
        0,
        [Event::Error {
            message: "Not logged in · Please run /login".into(),
        }],
    );
    cx.run_until_parked();
    assert_eq!(
        h.read(cx, |ws, _| ws.run_cards()),
        vec![(
            TaskKind::CopyEdit,
            "failed: Not logged in · Please run /login".to_string()
        )]
    );
}

/// A reply without the structured data, or with data of the wrong shape,
/// fails the card with a message instead of showing nothing.
#[gpui_kit::test]
fn a_reply_without_structured_data_fails_clearly(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "three");
    h.click(cx, "run-copy-edit");
    h.assistant.send(
        0,
        [Event::Done {
            text: "Here are my edits…".into(),
            data: None,
        }],
    );
    h.click(cx, "run-fact-check");
    h.assistant.send(
        1,
        [Event::Done {
            text: "".into(),
            data: Some(serde_json::json!({"summary": "x", "edits": []})),
        }],
    );
    cx.run_until_parked();
    let cards = h.read(cx, |ws, _| ws.run_cards());
    assert_eq!(
        cards[1],
        (
            TaskKind::CopyEdit,
            "failed: Couldn't read the reply: No suggestions came back".to_string()
        )
    );
    assert_eq!(cards[0].0, TaskKind::FactCheck);
    assert!(
        cards[0]
            .1
            .starts_with("failed: Couldn't read the reply: missing field `claims`"),
        "{cards:?}"
    );
}

#[gpui_kit::test]
fn fact_checks_stay_with_the_text_after_the_card_closes(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "three");
    h.click(cx, "run-fact-check");
    h.assistant.send(
        0,
        fact_check_reply(&[("three towers", "false"), ("in teh city", "supported")]),
    );
    cx.run_until_parked();
    assert_eq!(h.open_findings(cx), vec!["three towers", "in teh city"]);

    let run = h.read(cx, |ws, _| ws.run_ids()[0]);
    h.click(cx, ("close", run));
    assert!(h.read(cx, |ws, _| ws.run_cards()).is_empty());
    assert_eq!(h.open_findings(cx), vec!["three towers", "in teh city"]);
    let towers = h.finding_id(cx, "three towers");
    assert!(h.exists(cx, finding_button("dismiss", "kept", towers)));

    // With the cursor in a claim, it shows at the top too.
    h.cursor_at(cx, "towers");
    assert!(h.exists(cx, finding_button("dismiss", "cursor", towers)));
    h.cursor_at(cx, "second");
    assert!(!h.exists(cx, finding_button("dismiss", "cursor", towers)));

    // A new fact-check of the paragraph replaces the old results.
    h.cursor_at(cx, "three");
    h.press(cx, &shortcut("shift-c"));
    h.assistant
        .send(1, fact_check_reply(&[("three towers", "supported")]));
    cx.run_until_parked();
    assert_eq!(h.open_findings(cx), vec!["three towers"]);
}

#[gpui_kit::test]
fn references_insert_a_footnote_or_a_link(cx: &mut TestAppContext) {
    let doc = "Coffee improves memory. Tea is nice too.";
    let h = open(
        cx,
        Setup {
            text: doc,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "Coffee");
    h.press(cx, &shortcut("shift-r"));
    h.assistant
        .send(0, references_reply("Coffee improves memory."));
    cx.run_until_parked();
    let id = h.finding_id(cx, "Coffee improves memory.");
    h.click(cx, finding_button("footnote", "run", id));
    let text = h.text(cx);
    assert!(
        text.starts_with("Coffee improves memory.[^1] Tea"),
        "{text}"
    );
    assert!(
        text.contains("[^1]: Doe, J. (2020). *An Example Study*. Journal of Examples."),
        "{text}"
    );
    assert_eq!(h.notice(cx), "Added footnote [^1]");

    // Once inserted, it's done; undo takes the footnote back out.
    assert!(h.open_findings(cx).is_empty());
    h.press(cx, UNDO);
    assert_eq!(h.text(cx), doc);

    h.press(cx, &shortcut("shift-r"));
    h.assistant.send(1, references_reply("Tea is nice too."));
    cx.run_until_parked();
    let id = h.finding_id(cx, "Tea is nice too.");
    h.click(cx, finding_button("link", "run", id));
    let text = h.text(cx);
    assert!(
        text.contains("Tea is nice too ([An Example Study, 2020](https://example.com/study))."),
        "{text}"
    );
}

#[gpui_kit::test]
fn nothing_to_check_asks_for_a_passage(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    h.click(cx, "run-copy-edit");
    assert_eq!(h.assistant.request_count(), 0);
    assert_eq!(
        h.notice(cx),
        "Select some text, or put the cursor in a paragraph"
    );
}

#[gpui_kit::test]
fn without_the_assistant_the_buttons_are_disabled(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            assistant_fails: Some("Couldn't find wingman-service."),
            ..Default::default()
        },
    );
    let disabled = h.with_window(cx, |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window.find("run-copy-edit").disabled()
    });
    assert_eq!(disabled, Some(true));
    h.press(cx, &shortcut("e"));
    assert_eq!(h.assistant.request_count(), 0);
    assert_eq!(h.notice(cx), "The assistant isn't running");
    assert_eq!(
        h.read(cx, |ws, _| ws.service_status.label()),
        "Assistant unavailable"
    );
}

#[gpui_kit::test]
fn running_from_the_preview_goes_back_to_the_editor(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "three");
    h.click(cx, "mode-preview");
    assert_eq!(
        h.read(cx, |ws, _| ws.mode),
        crate::workspace::ViewMode::Preview
    );
    h.click(cx, "run-copy-edit");
    assert_eq!(h.read(cx, |ws, _| ws.mode), crate::workspace::ViewMode::Raw);
    assert_eq!(h.assistant.request_count(), 1);
}
