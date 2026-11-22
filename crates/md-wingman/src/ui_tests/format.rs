//! Formatting the selection: the shortcuts, the right-click menu's
//! commands, undo, and staying out of the way in the preview.

use gpui_kit::{Action, ClipboardItem, TestAppContext};

use super::*;
use crate::format::{context_menu_entries, MenuEntry};
use crate::workspace::ViewMode;

fn dispatch(h: &Harness, cx: &mut TestAppContext, action: impl Action) {
    h.update(cx, |_, window, cx| {
        window.dispatch_action(action.boxed_clone(), cx)
    });
}

/// The selected text.
fn selected(h: &Harness, cx: &mut TestAppContext) -> String {
    h.read(cx, |ws, cx| {
        let range = ws.editor().read(cx).selected_range();
        ws.text(cx)[range].to_string()
    })
}

#[gpui_kit::test]
fn shortcuts_style_the_selection_and_undo_in_one_step(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: "Make this loud.",
            ..Default::default()
        },
    );
    h.select(cx, "this");
    h.press(cx, &shortcut("b"));
    assert_eq!(h.text(cx), "Make **this** loud.");
    // The text stays selected, so styles stack.
    assert_eq!(selected(&h, cx), "this");
    h.press(cx, &shortcut("i"));
    assert_eq!(h.text(cx), "Make **_this_** loud.");
    h.press(cx, &shortcut("shift-x"));
    assert_eq!(h.text(cx), "Make **_~~this~~_** loud.");

    // Pressing one again takes it off.
    h.press(cx, &shortcut("i"));
    assert_eq!(h.text(cx), "Make **~~this~~** loud.");

    // Each style was one undo step.
    h.press(cx, UNDO);
    assert_eq!(h.text(cx), "Make **_~~this~~_** loud.");
    h.press(cx, UNDO);
    h.press(cx, UNDO);
    h.press(cx, UNDO);
    assert_eq!(h.text(cx), "Make this loud.");
}

#[gpui_kit::test]
fn the_link_shortcut_selects_what_to_type_next(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: "See the docs.",
            ..Default::default()
        },
    );
    h.select(cx, "the docs");
    h.press(cx, &shortcut("k"));
    assert_eq!(h.text(cx), "See [the docs](url).");
    assert_eq!(selected(&h, cx), "url");
    // Typing replaces the placeholder.
    h.type_text(cx, "https://example.com");
    assert_eq!(h.text(cx), "See [the docs](https://example.com).");

    // With a URL on the clipboard, it's filled in.
    while h.text(cx) != "See [the docs](url)." {
        h.press(cx, UNDO);
    }
    // The link itself is one step.
    h.press(cx, UNDO);
    assert_eq!(h.text(cx), "See the docs.");
    cx.write_to_clipboard(ClipboardItem::new_string("https://example.org".into()));
    h.select(cx, "docs");
    h.press(cx, &shortcut("k"));
    assert_eq!(h.text(cx), "See the [docs](https://example.org).");
}

#[gpui_kit::test]
fn paragraph_styles_apply_to_every_selected_line(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: "Title\n\nfirst\nsecond\n\nA thought.",
            ..Default::default()
        },
    );
    h.cursor_at(cx, "Title");
    dispatch(&h, cx, crate::Heading2);
    assert_eq!(h.text(cx), "## Title\n\nfirst\nsecond\n\nA thought.");
    dispatch(&h, cx, crate::Heading1);
    assert_eq!(h.text(cx), "# Title\n\nfirst\nsecond\n\nA thought.");

    h.select(cx, "first\nsec");
    dispatch(&h, cx, crate::NumberedList);
    assert_eq!(h.text(cx), "# Title\n\n1. first\n2. second\n\nA thought.");
    dispatch(&h, cx, crate::BulletedList);
    assert_eq!(h.text(cx), "# Title\n\n- first\n- second\n\nA thought.");

    h.cursor_at(cx, "thought");
    dispatch(&h, cx, crate::Quote);
    assert_eq!(h.text(cx), "# Title\n\n- first\n- second\n\n> A thought.");

    h.select(cx, "- first\n- second");
    dispatch(&h, cx, crate::CodeBlock);
    assert_eq!(
        h.text(cx),
        "# Title\n\n```\n- first\n- second\n```\n\n> A thought."
    );
    // The code is still selected, so again takes the fences off.
    dispatch(&h, cx, crate::CodeBlock);
    assert_eq!(h.text(cx), "# Title\n\n- first\n- second\n\n> A thought.");
}

#[gpui_kit::test]
fn formatting_waits_for_the_raw_editor(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: "Plain words.",
            ..Default::default()
        },
    );
    h.select(cx, "Plain");
    h.press(cx, &shortcut("p"));
    assert_eq!(h.read(cx, |ws, _| ws.mode), ViewMode::Preview);
    h.press(cx, &shortcut("b"));
    assert_eq!(h.text(cx), "Plain words.");
    assert_eq!(h.notice(cx), "Switch to Raw to format text");
    // It doesn't switch views on its own.
    assert_eq!(h.read(cx, |ws, _| ws.mode), ViewMode::Preview);
}

#[gpui_kit::test]
fn the_right_click_menu_offers_the_clipboard_and_styles(cx: &mut TestAppContext) {
    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                MenuEntry::Item(label, _) => label.to_string(),
                MenuEntry::Separator => "-".into(),
                MenuEntry::Submenu(label, items) => {
                    format!("{label} [{}]", labels(items).join(", "))
                }
            })
            .collect()
    }
    let entries = context_menu_entries();
    assert_eq!(
        labels(&entries),
        [
            "Cut",
            "Copy",
            "Paste",
            "-",
            "Bold",
            "Italic",
            "Strikethrough",
            "Code",
            "Link…",
            "-",
            "Paragraph [Heading 1, Heading 2, Heading 3, -, Quote, Bulleted List, Numbered List, Code Block]",
            "-",
            "Select All",
        ]
    );

    // Its items work on the selection when chosen, as a native menu would
    // dispatch them.
    let h = open(
        cx,
        Setup {
            text: "Copy this word.",
            ..Default::default()
        },
    );
    h.select(cx, "word");
    let MenuEntry::Item(_, bold) = &entries[4] else {
        panic!("Bold is an item")
    };
    let bold = bold.boxed_clone();
    h.update(cx, |_, window, cx| window.dispatch_action(bold, cx));
    assert_eq!(h.text(cx), "Copy this **word**.");
}
