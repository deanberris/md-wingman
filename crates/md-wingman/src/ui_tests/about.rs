//! The About window: opened from the app menu, showing the version,
//! contributors, license and links, and never in the way of the editor.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, VisualTestContext};

use super::*;
use crate::about::{self, OpenedLinks};

fn open_about(h: &Harness, cx: &mut TestAppContext) {
    h.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(crate::About), cx)
    });
    cx.run_until_parked();
}

fn about_window(cx: &mut TestAppContext) -> Option<AnyWindowHandle> {
    cx.update(|cx| about::window(cx))
}

/// Runs `f` on the About window, after a fresh frame.
fn in_about<R>(cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
    let handle = about_window(cx).expect("the About window is open");
    let result = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .expect("the About window is open");
    cx.run_until_parked();
    result
}

fn opened_links(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        cx.try_global::<OpenedLinks>()
            .map(|l| l.0.clone())
            .unwrap_or_default()
    })
}

#[gpui_kit::test]
fn about_opens_one_window_with_the_details(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    assert!(about_window(cx).is_none());

    open_about(&h, cx);
    assert_eq!(cx.windows().len(), 2);
    let shown = in_about(cx, |window, _| {
        [
            "about-version",
            "about-website",
            "about-github",
            "about-license",
        ]
        .into_iter()
        .all(|id| window.try_find(id).is_some_and(|e| e.visible()))
    });
    assert!(shown);
    let contributors = in_about(cx, |window, _| {
        window
            .try_find(gpui_kit::ElementId::NamedInteger("contributor".into(), 0))
            .is_some()
    });
    assert!(contributors);

    // Asking again brings the same window forward instead of a second one.
    open_about(&h, cx);
    assert_eq!(cx.windows().len(), 2);
}

#[gpui_kit::test]
fn about_links_open_the_website_github_and_license(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    open_about(&h, cx);
    for id in ["about-website", "about-github", "about-license"] {
        in_about(cx, |window, cx| window.click(id, cx));
    }
    assert_eq!(
        opened_links(cx),
        vec![
            about::WEBSITE.to_string(),
            about::REPOSITORY.to_string(),
            about::LICENSE_URL.to_string(),
        ]
    );
}

#[gpui_kit::test]
fn escape_closes_about_and_leaves_the_document_alone(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    h.type_text(cx, "Words in progress.");
    open_about(&h, cx);

    in_about(cx, |window, cx| window.press("escape", cx));
    assert!(about_window(cx).is_none());
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(h.text(cx), "Words in progress.");
    assert!(!cx.update(|cx| crate::quit::quit_requested(cx)));

    // It opens again after being closed.
    open_about(&h, cx);
    assert!(about_window(cx).is_some());
}

#[gpui_kit::test]
fn closing_the_editor_quits_even_with_about_open(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    h.type_text(cx, "Unsaved.");
    open_about(&h, cx);

    let mut vcx = VisualTestContext::from_window(h.window, cx);
    assert!(!vcx.simulate_close());
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(cx.update(|cx| crate::quit::quit_requested(cx)));
}
