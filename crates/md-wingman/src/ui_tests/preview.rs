//! The preview: one view that keeps up with the document, knows where the
//! source is in it, and can scroll there.

use gpui_kit::{TestAppContext, px};

use super::*;
use crate::preview::Align;

/// Enough paragraphs to fill the window several times over.
fn long_document() -> &'static str {
    let text: String = (0..300)
        .map(|i| format!("Paragraph number {i} of the long document.\n\n"))
        .collect();
    // `Setup` takes text that lives as long as the test.
    text.leak()
}

fn range_of(text: &str, needle: &str) -> std::ops::Range<usize> {
    let start = text.find(needle).expect("the needle is in the text");
    start..start + needle.len()
}

/// Draws frames until a requested scroll has been carried out.
fn settle(h: &Harness, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        h.frame(cx);
    }
}

#[gpui_kit::test]
fn the_preview_keeps_up_with_the_document(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: "# Title\n\nSome *text*.",
            ..Default::default()
        },
    );
    h.click(cx, "mode-preview");
    settle(&h, cx);
    assert_eq!(
        h.read(cx, |ws, cx| ws.preview_text(cx)).trim(),
        "Title\nSome text."
    );

    h.click(cx, "mode-raw");
    h.cursor_at_end(cx);
    h.type_text(cx, " And **more**.");
    h.click(cx, "mode-preview");
    settle(&h, cx);
    assert_eq!(
        h.read(cx, |ws, cx| ws.preview_text(cx)).trim(),
        "Title\nSome text. And more."
    );

    // A change made while the preview is showing reaches it too.
    h.update(cx, |ws, window, cx| {
        ws.set_text_for_test("Other words.", window, cx)
    });
    settle(&h, cx);
    assert_eq!(
        h.read(cx, |ws, cx| ws.preview_text(cx)).trim(),
        "Other words."
    );
}

#[gpui_kit::test]
fn a_place_in_the_source_is_found_in_the_preview(cx: &mut TestAppContext) {
    let text = "---\ntitle: Towers\n---\n\n# The *three* towers\n\nThey are **very** tall.\n";
    let h = open(
        cx,
        Setup {
            text,
            ..Default::default()
        },
    );
    h.click(cx, "mode-preview");
    settle(&h, cx);
    let rendered = h.read(cx, |ws, cx| ws.preview_text(cx));
    let find = |h: &Harness, cx: &mut TestAppContext, needle: &str| {
        h.read(cx, |ws, cx| ws.preview_range(range_of(text, needle), cx))
    };

    let very = find(&h, cx, "very").expect("the word is rendered");
    assert_eq!(&rendered[very], "very");
    // Markup around the words doesn't count.
    let heading = find(&h, cx, "# The *three* towers").expect("the heading is rendered");
    assert_eq!(&rendered[heading], "The three towers");
    // Front matter is drawn apart from the text, and markup alone renders
    // nothing.
    assert_eq!(find(&h, cx, "title: Towers"), None);
    assert_eq!(find(&h, cx, "**"), None);
}

#[gpui_kit::test]
fn the_preview_scrolls_to_a_place_in_the_source(cx: &mut TestAppContext) {
    let text = long_document();
    let h = open(
        cx,
        Setup {
            text,
            ..Default::default()
        },
    );
    h.click(cx, "mode-preview");
    settle(&h, cx);
    assert_eq!(h.read(cx, |ws, _| ws.preview_scrolled()), px(0.));

    let far = range_of(text, "Paragraph number 250 ");
    h.update(cx, |ws, _, cx| {
        ws.reveal_in_preview(far, Align::Nearest, cx)
    });
    settle(&h, cx);
    let down = h.read(cx, |ws, _| ws.preview_scrolled());
    assert!(down > px(1000.), "scrolled {down:?}");

    // And back up, no further than it takes to see the place.
    let near = range_of(text, "Paragraph number 100 ");
    h.update(cx, |ws, _, cx| {
        ws.reveal_in_preview(near, Align::Nearest, cx)
    });
    settle(&h, cx);
    let up = h.read(cx, |ws, _| ws.preview_scrolled());
    assert!(px(0.) < up && up < down, "scrolled {up:?} after {down:?}");
}

#[gpui_kit::test]
fn the_preview_opens_where_the_cursor_is(cx: &mut TestAppContext) {
    let text = long_document();
    let h = open(
        cx,
        Setup {
            text,
            ..Default::default()
        },
    );
    h.cursor_at(cx, "Paragraph number 250 ");
    h.click(cx, "mode-preview");
    settle(&h, cx);
    let down = h.read(cx, |ws, _| ws.preview_scrolled());
    assert!(down > px(1000.), "scrolled {down:?}");

    // From the top of the document, it opens at the top.
    h.click(cx, "mode-raw");
    h.cursor_at(cx, "Paragraph number 0 ");
    h.click(cx, "mode-preview");
    settle(&h, cx);
    assert_eq!(h.read(cx, |ws, _| ws.preview_scrolled()), px(0.));
}

/// A long document that cites one footnote near its top, and again further
/// down, with the footnote at its end.
fn footnoted_document() -> &'static str {
    let mut text = String::from("An opening claim.[^Source] And one with none.[^missing]\n\n");
    text.push_str(long_document());
    text.push_str("Cited again here.[^source]\n\n");
    text.push_str(long_document());
    text.push_str("[^source]: Where the *claim* comes from.\n");
    text.leak()
}

#[gpui_kit::test]
fn a_footnote_reference_jumps_to_its_footnote_and_back(cx: &mut TestAppContext) {
    let text = footnoted_document();
    let h = open(
        cx,
        Setup {
            text,
            ..Default::default()
        },
    );
    h.click(cx, "mode-preview");
    settle(&h, cx);
    assert_eq!(h.read(cx, |ws, _| ws.preview_scrolled()), px(0.));

    h.click(cx, "footnote-reference-source-0");
    settle(&h, cx);
    let at_footnote = h.read(cx, |ws, _| ws.preview_scrolled());
    assert!(at_footnote > px(5000.), "scrolled {at_footnote:?}");

    // The footnote has a way back to each place that cites it.
    assert!(h.exists(cx, "footnote-back-source-0"));
    assert!(h.exists(cx, "footnote-back-source-1"));
    assert!(!h.exists(cx, "footnote-back-source-2"));
    h.click(cx, "footnote-back-source-1");
    settle(&h, cx);
    let at_second = h.read(cx, |ws, _| ws.preview_scrolled());
    assert!(
        px(1000.) < at_second && at_second < at_footnote,
        "scrolled {at_second:?} after {at_footnote:?}"
    );

    h.click(cx, "footnote-reference-source-1");
    settle(&h, cx);
    h.click(cx, "footnote-back-source-0");
    settle(&h, cx);
    assert_eq!(h.read(cx, |ws, _| ws.preview_scrolled()), px(0.));
}

#[gpui_kit::test]
fn a_reference_without_a_footnote_is_plain_text(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: footnoted_document(),
            ..Default::default()
        },
    );
    h.click(cx, "mode-preview");
    settle(&h, cx);
    assert!(h.exists(cx, "footnote-reference-source-0"));
    assert!(!h.exists(cx, "footnote-reference-missing-0"));
    let rendered = h.read(cx, |ws, cx| ws.preview_text(cx));
    assert!(
        rendered.contains("And one with none.[^missing]"),
        "{:?}",
        &rendered[..80]
    );
}
