//! The window around the text: raw and preview, the ruler and reflow, the
//! resizable assistant pane, and dark mode and fonts.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{point, px, ElementId, TestAppContext};

use super::*;
use crate::settings::Appearance;
use crate::workspace::ViewMode;

#[gpui_kit::test]
fn raw_and_preview_toggle_from_the_toolbar_and_shortcut(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: "# Title\n\nSome *text*.",
            ..Default::default()
        },
    );
    assert_eq!(h.read(cx, |ws, _| ws.mode), ViewMode::Raw);
    h.click(cx, "mode-preview");
    assert_eq!(h.read(cx, |ws, _| ws.mode), ViewMode::Preview);
    h.click(cx, "mode-raw");
    assert_eq!(h.read(cx, |ws, _| ws.mode), ViewMode::Raw);
    h.press(cx, &shortcut("p"));
    assert_eq!(h.read(cx, |ws, _| ws.mode), ViewMode::Preview);
    h.press(cx, &shortcut("p"));
    assert_eq!(h.read(cx, |ws, _| ws.mode), ViewMode::Raw);

    // Back in the editor, typing goes into the document.
    h.cursor_at_end(cx);
    h.type_text(cx, " More.");
    assert_eq!(h.text(cx), "# Title\n\nSome *text*. More.");
}

#[gpui_kit::test]
fn typing_marks_the_document_changed_and_counts_words(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    assert_eq!(h.read(cx, |ws, _| ws.title()), "Untitled — md-wingman");
    h.type_text(cx, "Three short words");
    assert_eq!(h.text(cx), "Three short words");
    assert_eq!(h.read(cx, |ws, _| ws.title()), "Untitled • — md-wingman");
    assert_eq!(crate::document::word_count(&h.text(cx)), 3);
}

#[gpui_kit::test]
fn the_ruler_toggles_and_reflow_wraps_at_its_column(cx: &mut TestAppContext) {
    let settings = Settings {
        ruler_column: 20,
        ..Settings::default()
    };
    let h = open(
        cx,
        Setup {
            text: "one two three four five six seven eight nine ten\n\nNext.",
            settings,
            ..Default::default()
        },
    );
    h.update(cx, |_, window, cx| {
        window.dispatch_action(Box::new(crate::ToggleRuler), cx)
    });
    assert!(!h.read(cx, |ws, _| ws.settings.show_ruler));
    h.update(cx, |_, window, cx| {
        window.dispatch_action(Box::new(crate::ToggleRuler), cx)
    });
    assert!(h.read(cx, |ws, _| ws.settings.show_ruler));

    h.cursor_at(cx, "four");
    h.press(cx, &shortcut("alt-q"));
    assert_eq!(
        h.text(cx),
        "one two three four\nfive six seven eight\nnine ten\n\nNext."
    );
    assert_eq!(h.notice(cx), "Reflowed to 20 columns");
    // Reflowing again changes nothing.
    h.press(cx, &shortcut("alt-q"));
    assert_eq!(h.notice(cx), "Already wrapped");
    // And it's one undoable change.
    h.press(cx, UNDO);
    assert_eq!(
        h.text(cx),
        "one two three four five six seven eight nine ten\n\nNext."
    );
}

/// The split between the editor and the assistant, as the assistant's
/// share of the panes' width.
fn assistant_share(h: &Harness, cx: &mut TestAppContext) -> f32 {
    h.read(cx, |ws, cx| {
        let sizes = ws.split_sizes(cx);
        let total: f32 = sizes.iter().sum();
        sizes.last().copied().unwrap_or(0.) / total
    })
}

#[gpui_kit::test]
fn the_assistant_pane_opens_at_the_golden_ratio_and_remembers_a_drag(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    let golden = 1. - 1. / std::f32::consts::GOLDEN_RATIO;
    assert!(
        (assistant_share(&h, cx) - golden).abs() < 0.01,
        "{}",
        assistant_share(&h, cx)
    );

    // Drag the divider 200px to the left: the assistant gets wider, and the
    // workspace keeps its new share for next time.
    // The divider sits at the start of the assistant pane.
    let handle = ElementId::from(("resizable-handle", 0usize));
    let center = h.with_window(cx, |window, _| window.find_all(handle)[0].bounds().center());
    h.with_window(cx, |window, cx| {
        window.drag(center, point(center.x - px(200.), center.y), cx)
    });
    cx.run_until_parked();
    let share = assistant_share(&h, cx);
    assert!(share > golden + 0.05, "{share}");
    let saved = h.read(cx, |ws, _| ws.settings.assistant_fraction);
    assert!((saved - share).abs() < 0.01, "saved {saved}, shown {share}");
}

#[gpui_kit::test]
fn the_pane_keeps_a_saved_share(cx: &mut TestAppContext) {
    let settings = Settings {
        assistant_fraction: 0.5,
        ..Settings::default()
    };
    let h = open(
        cx,
        Setup {
            settings,
            ..Default::default()
        },
    );
    assert!((assistant_share(&h, cx) - 0.5).abs() < 0.01);
}

#[gpui_kit::test]
fn dark_mode_toggles_and_the_panel_picks_a_mode(cx: &mut TestAppContext) {
    let settings = Settings {
        appearance: Appearance::Light,
        ..Settings::default()
    };
    let h = open(
        cx,
        Setup {
            settings,
            ..Default::default()
        },
    );
    assert!(!cx.update(|cx| cx.theme().is_dark()));

    h.update(cx, |_, window, cx| {
        window.dispatch_action(Box::new(crate::ToggleDarkMode), cx)
    });
    assert!(cx.update(|cx| cx.theme().is_dark()));
    assert_eq!(h.read(cx, |ws, _| ws.settings.appearance), Appearance::Dark);

    // The Aa button opens the appearance panel; Light switches back.
    h.click(cx, "appearance-panel");
    assert!(h.exists(cx, "appearance-close"));
    h.click(cx, ("appearance", 1usize));
    assert!(!cx.update(|cx| cx.theme().is_dark()));
    assert_eq!(
        h.read(cx, |ws, _| ws.settings.appearance),
        Appearance::Light
    );
    h.click(cx, "appearance-close");
    assert!(!h.exists(cx, "appearance-close"));
}

#[gpui_kit::test]
fn a_saved_dark_appearance_applies_at_start(cx: &mut TestAppContext) {
    let settings = Settings {
        appearance: Appearance::Dark,
        ..Settings::default()
    };
    let _h = open(
        cx,
        Setup {
            settings,
            ..Default::default()
        },
    );
    assert!(cx.update(|cx| cx.theme().is_dark()));
}

#[gpui_kit::test]
fn font_size_changes_by_shortcut_and_panel(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    let mono = |cx: &mut TestAppContext| cx.update(|cx| f32::from(cx.theme().mono_font_size));
    assert_eq!(mono(cx), 14.);
    h.press(cx, &shortcut("="));
    assert_eq!(mono(cx), 15.);
    assert_eq!(h.notice(cx), "Editor font size 15");
    h.press(cx, &shortcut("-"));
    h.press(cx, &shortcut("-"));
    assert_eq!(mono(cx), 13.);
    h.press(cx, &shortcut("0"));
    assert_eq!(mono(cx), 14.);

    h.click(cx, "appearance-panel");
    h.click(cx, "size-up");
    assert_eq!(mono(cx), 15.);
    assert_eq!(h.read(cx, |ws, _| ws.settings.editor_font_size), 15.);
    h.click(cx, "size-down");
    assert_eq!(mono(cx), 14.);

    // Sizes stay in a readable range.
    for _ in 0..40 {
        h.press(cx, &shortcut("="));
    }
    assert_eq!(mono(cx), 32.);
}

/// Not Menlo: that is macOS's default, which gpui-component swaps for an
/// installed font, and the test platform has none.
#[gpui_kit::test]
fn a_saved_editor_font_applies_to_the_theme(cx: &mut TestAppContext) {
    let settings = Settings {
        editor_font: Some("JetBrains Mono".into()),
        editor_font_size: 18.,
        ..Settings::default()
    };
    let _h = open(
        cx,
        Setup {
            settings,
            ..Default::default()
        },
    );
    let (family, size) = cx.update(|cx| {
        let theme = cx.theme();
        (
            theme.mono_font_family.to_string(),
            f32::from(theme.mono_font_size),
        )
    });
    assert_eq!(family, "JetBrains Mono");
    assert_eq!(size, 18.);
}

#[gpui_kit::test]
fn a_blank_line_between_sentences_leaves_no_stray_space(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    h.type_text(cx, "Sentence.");
    h.press(cx, "enter");
    h.press(cx, "enter");
    h.type_text(cx, "Another sentence.");
    assert_eq!(h.text(cx), "Sentence.\n\nAnother sentence.");
}

#[gpui_kit::test]
fn a_blank_line_after_a_wrapped_paragraph_leaves_no_stray_space(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            width: 500.,
            ..Default::default()
        },
    );
    let para = "This is a long sentence that keeps going well past the edge of a narrow window so that it soft wraps onto several rows.";
    h.type_text(cx, para);
    h.press(cx, "enter");
    h.press(cx, "enter");
    h.type_text(cx, "Another sentence.");
    assert_eq!(h.text(cx), format!("{para}\n\nAnother sentence."));
}

#[gpui_kit::test]
fn a_dictated_phrase_after_a_blank_line_loses_its_leading_space(cx: &mut TestAppContext) {
    // Dictation apps paste each phrase, with a space in front when they guess
    // the cursor follows a word.
    let h = open(cx, Setup::default());
    h.type_text(cx, "Sentence.");
    h.press(cx, "enter");
    h.press(cx, "enter");
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
        " Another sentence.".into(),
    ));
    h.press(cx, &shortcut("v"));
    assert_eq!(h.text(cx), "Sentence.\n\nAnother sentence.");

    // Mid-line, the space is wanted.
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(" And more.".into()));
    h.press(cx, &shortcut("v"));
    assert_eq!(h.text(cx), "Sentence.\n\nAnother sentence. And more.");

    // It was one change: undo takes the whole phrase out.
    h.press(cx, UNDO);
    h.press(cx, UNDO);
    assert_eq!(h.text(cx), "Sentence.\n\n");
}
