//! Findings live and die with their text: through typing, deleting, undo
//! and redo.

use gpui_kit::TestAppContext;

use super::*;

const DOC: &str =
    "Their are three towers in teh city.\n\nAnother paragraph sits below it, unchanged.";

/// Opens the document and copy-edits its first paragraph.
fn with_suggestions(cx: &mut TestAppContext) -> Harness {
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
        copy_edit_reply(&[("Their are", "There are"), ("teh", "the")]),
    );
    cx.run_until_parked();
    assert_eq!(h.open_findings(cx), vec!["Their are", "teh"]);
    h
}

#[gpui_kit::test]
fn undoing_an_accept_brings_the_suggestion_back_and_redo_takes_it_away(cx: &mut TestAppContext) {
    let h = with_suggestions(cx);
    let teh = h.finding_id(cx, "teh");
    h.click(cx, finding_button("accept", "run", teh));
    assert_eq!(h.open_findings(cx), vec!["Their are"]);

    h.press(cx, UNDO);
    assert_eq!(h.text(cx), DOC);
    assert_eq!(h.open_findings(cx), vec!["Their are", "teh"]);
    assert!(h.exists(cx, finding_button("accept", "run", teh)));

    h.press(cx, REDO);
    assert!(h.text(cx).contains("in the city"));
    assert_eq!(h.open_findings(cx), vec!["Their are"]);
    assert!(!h.exists(cx, finding_button("accept", "run", teh)));
}

#[gpui_kit::test]
fn deleting_the_text_hides_a_finding_and_undo_restores_it(cx: &mut TestAppContext) {
    let h = with_suggestions(cx);
    // Select "teh" and delete it, by typing over it.
    h.select(cx, "teh ");
    h.press(cx, "backspace");
    assert_eq!(h.open_findings(cx), vec!["Their are"]);
    let teh = h.finding_id(cx, "teh");
    assert!(!h.exists(cx, finding_button("accept", "run", teh)));

    h.press(cx, UNDO);
    assert_eq!(h.text(cx), DOC);
    assert_eq!(h.open_findings(cx), vec!["Their are", "teh"]);
}

#[gpui_kit::test]
fn findings_follow_their_text_when_writing_above_it(cx: &mut TestAppContext) {
    let h = with_suggestions(cx);
    h.select_range(cx, 0..0);
    h.type_text(cx, "Intro. ");
    assert!(h.text(cx).starts_with("Intro. Their are"));
    assert_eq!(h.open_findings(cx), vec!["Their are", "teh"]);

    // Accepting still edits the right words after the text moved.
    let their = h.finding_id(cx, "Their are");
    h.click(cx, finding_button("accept", "run", their));
    assert!(
        h.text(cx)
            .starts_with("Intro. There are three towers in teh city.")
    );
}

#[gpui_kit::test]
fn a_dismissed_finding_stays_dismissed_through_undo(cx: &mut TestAppContext) {
    let h = with_suggestions(cx);
    let teh = h.finding_id(cx, "teh");
    h.click(cx, finding_button("dismiss", "run", teh));
    h.select(cx, "teh ");
    h.press(cx, "backspace");
    h.press(cx, UNDO);
    assert_eq!(h.text(cx), DOC);
    assert_eq!(h.open_findings(cx), vec!["Their are"]);
}

#[gpui_kit::test]
fn a_claim_follows_an_edit_accepted_inside_it(cx: &mut TestAppContext) {
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
        fact_check_reply(&[("three towers in teh city", "supported")]),
    );
    h.click(cx, "run-copy-edit");
    h.assistant.send(1, copy_edit_reply(&[("teh", "the")]));
    cx.run_until_parked();

    let teh = h.finding_id(cx, "teh");
    h.click(cx, finding_button("accept", "run", teh));
    // The claim now quotes the corrected words, and is still shown.
    assert!(
        h.open_findings(cx)
            .contains(&"three towers in the city".to_string())
    );

    // Undo the edit: the claim goes back to the old wording.
    h.press(cx, UNDO);
    let open = h.open_findings(cx);
    assert!(
        open.contains(&"three towers in teh city".to_string()),
        "{open:?}"
    );
}

#[gpui_kit::test]
fn closing_a_card_drops_its_edits(cx: &mut TestAppContext) {
    let h = with_suggestions(cx);
    let run = h.read(cx, |ws, _| ws.run_ids()[0]);
    h.click(cx, ("close", run));
    assert!(h.open_findings(cx).is_empty());
    assert_eq!(h.text(cx), DOC);
}

#[gpui_kit::test]
fn a_suggestion_that_cannot_be_found_says_its_text_changed(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: DOC,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "three");
    h.click(cx, "run-copy-edit");
    // The writer keeps typing while the assistant works, and rewrites the words.
    h.select(cx, "teh");
    h.type_text(cx, "the");
    h.assistant.send(0, copy_edit_reply(&[("teh", "the")]));
    cx.run_until_parked();
    assert!(h.open_findings(cx).is_empty());
    let run = h.read(cx, |ws, _| ws.run_ids()[0]);
    // No accept button for it, and the card stays closable.
    let teh = h.finding_id(cx, "teh");
    assert!(!h.exists(cx, finding_button("accept", "run", teh)));
    assert!(h.exists(cx, ("close", run)));
}
