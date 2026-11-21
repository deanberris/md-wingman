//! Reviewing the whole document: Jev's suggestion after a long pause, which
//! never starts a review by itself, and the review's chunks and findings.

use gpui_kit::TestAppContext;
use wingman_client::TaskKind;

use super::*;
use crate::review::READINESS_PAUSE;

const PAUSE: u64 = 3_000;

/// A finished-looking post: three paragraphs, over 150 words.
const POST: &str = "# Towers\n\nThe Eiffel Tower was finished in 1889 for the World's Fair in Paris, and for four decades it was the tallest structure anyone had built. Critics hated it at first, and a petition signed by artists called it useless and monstrous. Gustave Eiffel's company designed and built it in a little over two years, which was fast even then.\n\n## Why it stayed\n\nIt was meant to come down after twenty years. It survived because it turned out to be useful as a radio mast, and the army found it handy for sending messages a long way. By the time the permit ran out, nobody wanted it gone.\n\n## Today\n\nNow it is the most visited paid monument in the world, and teh view from the top is worth the queue. Go early in the morning, book ahead, and take the stairs to the second floor if your legs allow it, because the climb is half the fun.\n";

fn settings(jev: bool) -> Settings {
    Settings {
        continuous: true,
        continuous_delay_ms: PAUSE,
        continuous_fact_check: false,
        jev_prefilter: jev,
        ..Settings::default()
    }
}

fn answer_everything(h: &Harness) {
    h.assistant.answer_with(|kind, text| match kind {
        TaskKind::CopyEdit if text.contains("teh") => copy_edit_reply(&[("teh", "the")]),
        TaskKind::CopyEdit => copy_edit_reply(&[]),
        TaskKind::FactCheck if text.contains("1889") => {
            fact_check_reply(&[("finished in 1889", "supported")])
        }
        TaskKind::FactCheck => fact_check_reply(&[]),
        TaskKind::FindReferences if text.contains("most visited") => {
            references_reply("the most visited paid monument in the world")
        }
        TaskKind::FindReferences => done(
            "none",
            serde_json::json!({"summary": "none", "references": []}),
        ),
    });
}

fn long_pause(h: &Harness, cx: &mut TestAppContext) {
    h.wait(cx, READINESS_PAUSE.as_millis() as u64 + 10);
}

#[gpui_kit::test]
fn suggests_a_review_once_the_draft_looks_ready_but_never_starts_one(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: POST,
            settings: settings(true),
            ..Default::default()
        },
    );
    h.jev.readiness_with(|_| Ok(0.8));
    h.cursor_at_end(cx);
    h.type_text(cx, "\nThe end.");

    // Not after the usual pause, only after a long one.
    h.wait(cx, PAUSE + 10);
    assert_eq!(h.jev.readiness_calls(), 0);
    assert!(!h.exists(cx, "review-start"));
    long_pause(&h, cx);
    assert_eq!(h.jev.readiness_calls(), 1);
    assert!(h.exists(cx, "review-start"));

    // It only suggests: nothing is sent to Claude, however long it waits.
    h.wait(cx, 120_000);
    assert_eq!(h.assistant.request_count(), 0);
    assert_eq!(
        h.jev.readiness_calls(),
        1,
        "nothing changed, so no new grade"
    );

    // "Not now" hides it, and a small edit doesn't bring it back.
    h.click(cx, "review-later");
    assert!(!h.exists(cx, "review-start"));
    h.type_text(cx, " Really.");
    long_pause(&h, cx);
    assert_eq!(h.jev.readiness_calls(), 1);
    assert!(!h.exists(cx, "review-start"));

    // Writing more does, when Jev still thinks it's ready.
    h.type_text(
        cx,
        "\n\nA whole new closing paragraph, with enough words in it to count as a real change to the post, and then some more words after that, just to be sure it crosses the line for a new grade from Jev. It goes on a little longer still, so that nobody could mistake it for a small edit to a sentence.",
    );
    long_pause(&h, cx);
    assert_eq!(h.jev.readiness_calls(), 2);
    assert!(h.exists(cx, "review-start"));
}

#[gpui_kit::test]
fn no_suggestion_for_an_unfinished_draft(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: POST,
            settings: settings(true),
            ..Default::default()
        },
    );
    h.jev.readiness_with(|_| Ok(0.5));
    h.cursor_at_end(cx);
    h.type_text(cx, "\nTODO: an ending.");
    long_pause(&h, cx);
    assert_eq!(h.jev.readiness_calls(), 1);
    assert!(!h.exists(cx, "review-start"));
}

#[gpui_kit::test]
fn no_grading_without_the_prefilter_or_for_a_few_lines(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: POST,
            settings: settings(false),
            ..Default::default()
        },
    );
    h.jev.readiness_with(|_| Ok(0.9));
    h.cursor_at_end(cx);
    h.type_text(cx, "\nThe end.");
    long_pause(&h, cx);
    assert_eq!(h.jev.readiness_calls(), 0);

    let h = open(
        cx,
        Setup {
            text: "A single paragraph, nowhere near long enough to review.",
            settings: settings(true),
            ..Default::default()
        },
    );
    h.jev.readiness_with(|_| Ok(0.9));
    h.cursor_at_end(cx);
    h.type_text(cx, " More.");
    long_pause(&h, cx);
    assert_eq!(h.jev.readiness_calls(), 0);
}

#[gpui_kit::test]
fn the_suggestions_button_reviews_the_whole_document(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: POST,
            settings: settings(true),
            ..Default::default()
        },
    );
    answer_everything(&h);
    h.jev.readiness_with(|_| Ok(0.8));
    h.cursor_at_end(cx);
    h.type_text(cx, "\nThe end.");
    long_pause(&h, cx);
    h.click(cx, "review-start");

    // One request for each kind of help, each with the whole document from
    // its first paragraph on, and no context since that's all of it.
    let requests = h.assistant.requests();
    let kinds: Vec<TaskKind> = requests.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        kinds,
        vec![
            TaskKind::CopyEdit,
            TaskKind::FactCheck,
            TaskKind::FindReferences
        ]
    );
    for (i, (_, text)) in requests.iter().enumerate() {
        assert!(text.starts_with("The Eiffel Tower"), "{text}");
        assert!(
            text.contains("## Today") && text.ends_with("half the fun."),
            "{text:?}"
        );
        assert_eq!(h.assistant.context(i), None);
    }

    // Findings anchored to their text, in the document's lists.
    let mut found = h.open_findings(cx);
    found.sort();
    assert_eq!(
        found,
        vec![
            "finished in 1889",
            "teh",
            "the most visited paid monument in the world"
        ]
    );
    let teh = h.finding_id(cx, "teh");
    assert!(h.exists(cx, finding_button("accept", "written", teh)));
    assert!(h.read(cx, |ws, _| ws.run_cards()).is_empty());
    assert_eq!(
        h.read(cx, |ws, _| ws.review.outcome().map(|o| o.findings)),
        Some(3)
    );
    assert!(h.exists(cx, "review-close"));
    assert!(!h.exists(cx, "review-start"));

    // Checking as you write doesn't do the same paragraphs again.
    h.wait(cx, PAUSE * 3);
    assert_eq!(h.assistant.request_count(), 3);
}

#[gpui_kit::test]
fn a_long_document_is_reviewed_in_chunks(cx: &mut TestAppContext) {
    let paragraph = |n: usize| {
        format!("Paragraph {n} says something. ")
            .repeat(75)
            .trim()
            .to_string()
    };
    let doc: String = (0..10).map(|n| paragraph(n) + "\n\n").collect::<String>();
    let doc: &'static str = Box::leak(doc.into_boxed_str());
    let h = open(
        cx,
        Setup {
            text: doc,
            settings: Settings::default(),
            ..Default::default()
        },
    );
    answer_everything(&h);
    // Started from the menu's shortcut, with no suggestion needed.
    h.press(cx, &shortcut("shift-d"));

    let requests = h.assistant.requests();
    let count = |kind| requests.iter().filter(|(k, _)| *k == kind).count();
    // About 2,100 bytes a paragraph: copy-edit takes up to 16,000 bytes at a
    // time, fact-checks and references 6,000.
    assert_eq!(count(TaskKind::CopyEdit), 2);
    assert_eq!(count(TaskKind::FactCheck), 5);
    assert_eq!(count(TaskKind::FindReferences), 5);
    // Every paragraph, once for each kind.
    for kind in [
        TaskKind::CopyEdit,
        TaskKind::FactCheck,
        TaskKind::FindReferences,
    ] {
        let sent: String = requests
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, t)| t.clone())
            .collect();
        for n in 0..10 {
            assert_eq!(sent.matches(&paragraph(n)).count(), 1, "{kind:?} {n}");
        }
    }
    // Each chunk comes with the rest of the document for context.
    assert!(h.assistant.context(1).is_some_and(|c| c.len() == doc.len()));
    assert_eq!(
        h.read(cx, |ws, _| ws.review.outcome().map(|o| o.parts)),
        Some(12)
    );
}

#[gpui_kit::test]
fn a_paragraph_edited_during_the_review_is_reviewed_again(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: POST,
            settings: Settings::default(),
            ..Default::default()
        },
    );
    h.press(cx, &shortcut("shift-d"));
    assert_eq!(h.assistant.request_count(), 3);
    assert!(h.exists(cx, "review-cancel"));
    assert!(h.read(cx, |ws, _| ws.review.is_running()));

    // Writing on while it runs: the runs on the old text are dropped, and
    // the review starts again on the new text.
    h.cursor_at(cx, "Go early");
    h.type_text(cx, "Really, ");
    for i in 0..3 {
        assert!(h.assistant.was_cancelled(i), "request {i}");
    }
    let requests = h.assistant.requests();
    assert_eq!(requests.len(), 6);
    assert!(requests[3..]
        .iter()
        .all(|(_, t)| t.contains("Really, Go early")));

    // Its findings land on the new text.
    h.assistant.send(3, copy_edit_reply(&[("teh", "the")]));
    h.assistant.send(4, fact_check_reply(&[]));
    h.assistant.send(
        5,
        done(
            "none",
            serde_json::json!({"summary": "none", "references": []}),
        ),
    );
    h.wait(cx, 0);
    assert_eq!(h.open_findings(cx), vec!["teh"]);
    assert!(!h.read(cx, |ws, _| ws.review.is_running()));
    assert_eq!(
        h.read(cx, |ws, _| ws.review.outcome().cloned())
            .map(|o| (o.parts, o.failed)),
        Some((3, 0))
    );
}

#[gpui_kit::test]
fn stopping_the_review_cancels_its_runs(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: POST,
            settings: Settings::default(),
            ..Default::default()
        },
    );
    h.press(cx, &shortcut("shift-d"));
    h.click(cx, "review-cancel");
    for i in 0..3 {
        assert!(h.assistant.was_cancelled(i));
    }
    assert_eq!(h.assistant.request_count(), 3);
    assert!(!h.read(cx, |ws, _| ws.review.is_running()));
    assert!(!h.exists(cx, "review-cancel"));
}
