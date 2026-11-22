//! The Markdown quick reference: opened only when asked, and never taking
//! the focus from the writing.

use gpui_kit::TestAppContext;

use super::*;
use crate::format::{Inline, insert_link, toggle_inline};
use crate::guide::SECTIONS;

#[gpui_kit::test]
fn the_guide_opens_on_request_and_leaves_the_cursor_in_the_text(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: "Draft",
            ..Default::default()
        },
    );
    // Not there until asked for, even after writing.
    h.cursor_at_end(cx);
    h.type_text(cx, " text");
    assert!(!h.exists(cx, "guide-close"));

    h.press(cx, &shortcut("/"));
    assert!(h.exists(cx, "guide-close"));
    // Typing still goes into the document.
    h.type_text(cx, " more");
    assert_eq!(h.text(cx), "Draft text more");
    assert!(h.exists(cx, "guide-close"));

    // The shortcut again, or ×, puts it away.
    h.press(cx, &shortcut("/"));
    assert!(!h.exists(cx, "guide-close"));
    h.click(cx, "markdown-guide");
    assert!(h.exists(cx, "guide-close"));
    h.click(cx, "guide-close");
    assert!(!h.exists(cx, "guide-close"));
    // Clicking the buttons didn't move the cursor out of the text.
    h.type_text(cx, ".");
    assert_eq!(h.text(cx), "Draft text more.");
}

#[gpui_kit::test]
fn the_guide_shows_what_the_format_commands_type(_cx: &mut TestAppContext) {
    let syntax = |meaning: &str| {
        SECTIONS
            .iter()
            .flat_map(|(_, entries)| entries.iter())
            .find(|e| e.meaning == meaning)
            .map(|e| e.syntax)
            .unwrap_or_else(|| panic!("{meaning} is in the guide"))
    };
    let styled = |word: &str, style| {
        let edit = toggle_inline(word, 0..word.len(), style);
        edit.text
    };
    assert_eq!(syntax("Bold"), styled("bold", Inline::Bold));
    assert_eq!(syntax("Italic"), styled("italic", Inline::Italic));
    assert_eq!(
        syntax("Strikethrough"),
        styled("struck", Inline::Strikethrough)
    );
    assert_eq!(syntax("Code in a sentence"), styled("code", Inline::Code));
    let link = insert_link("text", 0..4, Some("https://…"));
    assert_eq!(syntax("Link"), link.text);
}
