//! Checking as you write: the pause before a check, which paragraphs get
//! checked, and the Jev pre-filter in front of them.

use gpui_kit::TestAppContext;
use wingman_client::TaskKind;

use super::*;
use crate::jev::JevError;

const PAUSE: u64 = 3_000;
const FIRST: &str = "The first paragraph was already here when the mode came on.";

fn settings(jev: bool) -> Settings {
    Settings {
        continuous: true,
        continuous_delay_ms: PAUSE,
        continuous_fact_check: false,
        jev_prefilter: jev,
        ..Settings::default()
    }
}

/// Copy-edits every paragraph it's sent with one suggestion, "teh".
fn answer_copy_edits(h: &Harness) {
    h.assistant.answer_with(|kind, text| match kind {
        TaskKind::CopyEdit if text.contains("teh") => copy_edit_reply(&[("teh", "the")]),
        TaskKind::CopyEdit => copy_edit_reply(&[]),
        TaskKind::FactCheck => fact_check_reply(&[]),
        TaskKind::FindReferences => done(
            "none",
            serde_json::json!({"summary": "none", "references": []}),
        ),
    });
}

/// Starts writing a second paragraph after the first.
fn write_second(h: &Harness, cx: &mut TestAppContext, text: &str) {
    h.cursor_at_end(cx);
    h.type_text(cx, "\n\n");
    h.type_text(cx, text);
}

#[gpui_kit::test]
fn checks_a_new_paragraph_once_typing_pauses(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(false),
            ..Default::default()
        },
    );
    answer_copy_edits(&h);
    write_second(&h, cx, "Then I wrote teh second one, and it ends here.");

    // Nothing while typing, nor just before the pause is up.
    assert_eq!(h.assistant.request_count(), 0);
    h.wait(cx, PAUSE - 100);
    assert_eq!(h.assistant.request_count(), 0);

    h.wait(cx, 200);
    // Only the new paragraph; the one already there counts as checked.
    assert_eq!(
        h.assistant.requests(),
        vec![(
            TaskKind::CopyEdit,
            "Then I wrote teh second one, and it ends here.".to_string()
        )]
    );
    // Its suggestion is listed under "Suggested as you write", with no card.
    assert!(h.read(cx, |ws, _| ws.run_cards()).is_empty());
    assert_eq!(h.open_findings(cx), vec!["teh"]);
    let teh = h.finding_id(cx, "teh");
    assert!(h.exists(cx, finding_button("accept", "written", teh)));

    // Accepting it doesn't count as new writing to check again.
    h.click(cx, finding_button("accept", "written", teh));
    h.wait(cx, PAUSE * 2);
    assert_eq!(h.assistant.request_count(), 1);
}

#[gpui_kit::test]
fn each_keystroke_restarts_the_pause(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(false),
            ..Default::default()
        },
    );
    answer_copy_edits(&h);
    write_second(&h, cx, "A second paragraph that I keep on writing.");
    for _ in 0..5 {
        h.wait(cx, PAUSE / 2);
        h.type_text(cx, " More.");
    }
    assert_eq!(h.assistant.request_count(), 0);
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.assistant.request_count(), 1);

    // Nothing changed since: no more checks, however long the pause.
    h.wait(cx, PAUSE * 10);
    assert_eq!(h.assistant.request_count(), 1);
}

#[gpui_kit::test]
fn waits_for_the_paragraph_being_written_to_end_a_sentence(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(false),
            ..Default::default()
        },
    );
    answer_copy_edits(&h);
    write_second(
        &h,
        cx,
        "Dictation often pauses in the middle of a thought and",
    );
    h.wait(cx, PAUSE * 2);
    assert_eq!(h.assistant.request_count(), 0);

    h.type_text(cx, " then finishes it.");
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.assistant.request_count(), 1);
}

#[gpui_kit::test]
fn editing_a_paragraph_while_it_is_checked_gives_up_on_that_check(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(false),
            ..Default::default()
        },
    );
    // No responder: the check stays in flight until answered.
    write_second(&h, cx, "This paragraph is being checked right now, I hope.");
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.assistant.request_count(), 1);
    assert!(h.read(cx, |ws, _| ws.continuous.is_checking()));

    h.cursor_at(cx, "being");
    h.type_text(cx, "really ");
    assert!(h.assistant.was_cancelled(0));
    assert!(!h.read(cx, |ws, _| ws.continuous.is_checking()));

    // The new wording is checked after the next pause.
    answer_copy_edits(&h);
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.assistant.request_count(), 2);
    assert!(h.assistant.requests()[1].1.contains("really being checked"));
}

#[gpui_kit::test]
fn the_button_and_shortcut_turn_it_on_and_off(cx: &mut TestAppContext) {
    let mut s = settings(false);
    s.continuous = false;
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: s,
            ..Default::default()
        },
    );
    answer_copy_edits(&h);

    write_second(
        &h,
        cx,
        "Written while the mode is off, so nobody checks it.",
    );
    h.wait(cx, PAUSE * 2);
    assert_eq!(h.assistant.request_count(), 0);

    h.click(cx, "continuous");
    assert!(h.read(cx, |ws, _| ws.settings.continuous));
    assert_eq!(h.notice(cx), "Checking paragraphs as you write");
    // What was written before it came on isn't checked.
    h.wait(cx, PAUSE * 2);
    assert_eq!(h.assistant.request_count(), 0);

    h.press(cx, &shortcut("shift-k"));
    assert!(!h.read(cx, |ws, _| ws.settings.continuous));
    assert_eq!(h.notice(cx), "Stopped checking as you write");
}

#[gpui_kit::test]
fn a_failed_check_pauses_with_the_reason(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(false),
            ..Default::default()
        },
    );
    h.assistant.answer_with(|_, _| {
        vec![wingman_client::Event::Error {
            message: "Claude stopped early (error_max_turns)".into(),
        }]
    });
    write_second(
        &h,
        cx,
        "A paragraph the assistant will fail to check today.",
    );
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.assistant.request_count(), 1);
    assert_eq!(
        h.notice(cx),
        "Paused checking as you write: Claude stopped early (error_max_turns)"
    );
}

// The Jev pre-filter.

#[gpui_kit::test]
fn jev_grades_first_and_only_high_grades_are_checked(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(true),
            ..Default::default()
        },
    );
    answer_copy_edits(&h);
    // Copy-edit scores high, references low: only a copy-edit runs.
    h.jev.grade_with(|_| Ok(grades(0.9, 0.1, 0.2)));
    write_second(&h, cx, "Then I wrote teh second one, and it ends here.");
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.jev.calls(), 1);
    assert_eq!(
        h.assistant.requests(),
        vec![(
            TaskKind::CopyEdit,
            "Then I wrote teh second one, and it ends here.".to_string()
        )]
    );
    // Graded once: the same paragraph isn't graded again.
    h.wait(cx, PAUSE * 3);
    assert_eq!(h.jev.calls(), 1);
    assert_eq!(h.assistant.request_count(), 1);
}

#[gpui_kit::test]
fn nothing_above_the_threshold_means_no_requests(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(true),
            ..Default::default()
        },
    );
    answer_copy_edits(&h);
    // Just under the default 66% bar.
    h.jev.grade_with(|_| Ok(grades(0.65, 0.5, 0.6)));
    write_second(
        &h,
        cx,
        "A clean paragraph with nothing much to check in it.",
    );
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.jev.calls(), 1);
    assert_eq!(h.assistant.request_count(), 0);
}

#[gpui_kit::test]
fn references_are_found_only_with_the_prefilter(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(true),
            ..Default::default()
        },
    );
    answer_copy_edits(&h);
    h.jev.grade_with(|_| Ok(grades(0.1, 0.1, 0.9)));
    write_second(
        &h,
        cx,
        "According to a 2020 study, most people drink coffee.",
    );
    h.wait(cx, PAUSE + 10);
    assert_eq!(
        h.assistant.requests(),
        vec![(
            TaskKind::FindReferences,
            "According to a 2020 study, most people drink coffee.".to_string()
        )]
    );
}

#[gpui_kit::test]
fn without_a_key_it_asks_for_one_and_stops(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(true),
            jev_key: None,
            ..Default::default()
        },
    );
    answer_copy_edits(&h);
    write_second(&h, cx, "Written with the pre-filter on but no key to use.");
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.jev.calls(), 0);
    assert_eq!(h.assistant.request_count(), 0);
    assert!(h
        .notice(cx)
        .starts_with("The Jev pre-filter needs a TypeSafe API key"));

    // Saving a key in the panel unblocks it.
    h.update(cx, |_, window, cx| {
        window.dispatch_action(Box::new(crate::SetJevKey), cx)
    });
    assert!(h.exists(cx, "key-save"));
    h.type_text(cx, "sk-test");
    h.click(cx, "key-save");
    assert_eq!(h.jev.stored_key().as_deref(), Some("sk-test"));
    assert!(!h.exists(cx, "key-save"));
    h.jev.grade_with(|_| Ok(grades(0.9, 0., 0.)));
    h.cursor_at_end(cx);
    h.type_text(cx, " Then more.");
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.jev.calls(), 1);
    assert_eq!(h.assistant.request_count(), 1);
}

#[gpui_kit::test]
fn turning_jev_on_without_a_key_opens_the_key_panel(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(false),
            jev_key: None,
            ..Default::default()
        },
    );
    h.click(cx, "jev-toggle");
    assert!(h.read(cx, |ws, _| ws.settings.jev_prefilter));
    assert!(h.exists(cx, "key-save"));
    h.click(cx, "key-cancel");
    assert!(!h.exists(cx, "key-save"));
    h.click(cx, "jev-toggle");
    assert!(!h.read(cx, |ws, _| ws.settings.jev_prefilter));
    assert_eq!(h.notice(cx), "Jev pre-filter off");
}

#[gpui_kit::test]
fn a_rejected_key_stops_grading(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(true),
            ..Default::default()
        },
    );
    h.jev.grade_with(|_| Err(JevError::BadKey));
    write_second(&h, cx, "Graded with a key that TypeSafe no longer accepts.");
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.jev.calls(), 1);
    assert!(h
        .notice(cx)
        .starts_with("TypeSafe didn't accept the API key"));
    h.cursor_at_end(cx);
    h.type_text(cx, " More.");
    h.wait(cx, PAUSE * 3);
    assert_eq!(h.jev.calls(), 1);
}

#[gpui_kit::test]
fn a_busy_jev_is_retried_after_a_backoff(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: FIRST,
            settings: settings(true),
            ..Default::default()
        },
    );
    answer_copy_edits(&h);
    h.jev.queue(Err(JevError::Busy {
        retry_after: Some(10),
    }));
    h.jev.grade_with(|_| Ok(grades(0.9, 0., 0.)));
    write_second(
        &h,
        cx,
        "Graded while TypeSafe is busy, then again later on.",
    );
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.jev.calls(), 1);
    assert_eq!(h.notice(cx), "Jev is busy; trying again in 10s");
    assert_eq!(h.assistant.request_count(), 0);

    h.wait(cx, 9_000);
    assert_eq!(h.jev.calls(), 1);
    h.wait(cx, 1_100);
    assert_eq!(h.jev.calls(), 2);
    assert_eq!(h.assistant.request_count(), 1);
}
