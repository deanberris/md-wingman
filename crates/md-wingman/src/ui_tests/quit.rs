//! Asking to save unsaved changes before quitting or closing the window.

use gpui_kit::{TestAppContext, VisualTestContext};

use super::*;

const PROMPT_DETAIL: &str = "Your changes will be lost if you don't save them.";

fn quit_requested(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| crate::quit::quit_requested(cx))
}

/// An untitled document with a change in it.
fn edited(cx: &mut TestAppContext) -> Harness {
    let h = open(cx, Setup::default());
    h.type_text(cx, "Unsaved words.");
    assert!(h.read(cx, |ws, _| ws.title()).contains('•'));
    h
}

#[gpui_kit::test]
fn quitting_with_nothing_unsaved_quits_without_asking(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    h.press(cx, &shortcut("q"));
    assert!(!cx.has_pending_prompt());
    assert!(quit_requested(cx));
}

#[gpui_kit::test]
fn quitting_with_changes_asks_and_cancel_keeps_them(cx: &mut TestAppContext) {
    let h = edited(cx);
    h.press(cx, &shortcut("q"));
    assert_eq!(
        cx.pending_prompt(),
        Some((
            "Do you want to save the changes you made to “Untitled”?".to_string(),
            PROMPT_DETAIL.to_string()
        ))
    );
    assert!(!quit_requested(cx));

    // A second ⌘Q while asking doesn't stack another prompt.
    h.press(cx, &shortcut("q"));
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert!(!quit_requested(cx));
    assert_eq!(h.text(cx), "Unsaved words.");

    // Asking again works after a cancel.
    h.press(cx, &shortcut("q"));
    assert!(cx.has_pending_prompt());
}

#[gpui_kit::test]
fn dont_save_quits_without_saving(cx: &mut TestAppContext) {
    let h = edited(cx);
    h.press(cx, &shortcut("q"));
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path());
    assert!(quit_requested(cx));
    assert_eq!(h.read(cx, |ws, _| ws.document_path()), None);
}

#[gpui_kit::test]
fn save_on_an_untitled_document_asks_where_then_quits(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("post.md");
    let h = edited(cx);
    h.press(cx, &shortcut("q"));
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path());
    assert!(!quit_requested(cx));

    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Unsaved words.");
    assert_eq!(h.read(cx, |ws, _| ws.document_path()), Some(target));
    assert!(quit_requested(cx));
}

#[gpui_kit::test]
fn cancelling_the_save_dialog_cancels_the_quit(cx: &mut TestAppContext) {
    let h = edited(cx);
    h.press(cx, &shortcut("q"));
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert!(!quit_requested(cx));
    assert!(h.read(cx, |ws, _| ws.title()).contains('•'));
    assert_eq!(h.text(cx), "Unsaved words.");
}

#[gpui_kit::test]
fn save_on_an_open_file_writes_it_in_place_then_quits(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("draft.md");
    std::fs::write(&file, "Draft.").unwrap();
    let h = open(cx, Setup::default());
    let path = file.clone();
    h.update(cx, |ws, window, cx| ws.open_path(path, window, cx));
    h.cursor_at_end(cx);
    h.type_text(cx, " More.");

    h.press(cx, &shortcut("q"));
    assert_eq!(
        cx.pending_prompt().map(|(message, _)| message),
        Some("Do you want to save the changes you made to “draft.md”?".to_string())
    );
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Draft. More.");
    assert!(quit_requested(cx));
}

#[gpui_kit::test]
fn closing_the_window_asks_first(cx: &mut TestAppContext) {
    let h = edited(cx);
    let mut vcx = VisualTestContext::from_window(h.window, cx);
    assert!(!vcx.simulate_close());
    assert!(cx.has_pending_prompt());

    // Cancel leaves the window open.
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert!(!quit_requested(cx));

    // Don't Save closes it, and with no window left the app quits.
    let mut vcx = VisualTestContext::from_window(h.window, cx);
    assert!(!vcx.simulate_close());
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(cx.windows().is_empty());
    assert!(quit_requested(cx));
}

#[gpui_kit::test]
fn closing_a_saved_window_closes_at_once(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    let mut vcx = VisualTestContext::from_window(h.window, cx);
    assert!(vcx.simulate_close());
    assert!(!cx.has_pending_prompt());
}
