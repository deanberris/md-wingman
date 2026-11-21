//! The setup wizard: shown at first launch beside the editor, never in the
//! way of typing, with commands to copy or run for Claude Code and Jev.

use gpui_kit::TestAppContext;

use super::*;

fn first_run(claude: ClaudeStatus) -> Setup {
    Setup {
        first_run: true,
        claude,
        jev_key: None,
        ..Default::default()
    }
}

fn signed_out() -> ClaudeStatus {
    ClaudeStatus::SignedOut {
        path: "/usr/local/bin/claude".into(),
    }
}

fn claude_ready(h: &Harness, cx: &mut TestAppContext) -> bool {
    h.read(cx, |ws, _| {
        ws.setup
            .as_ref()
            .and_then(|w| w.claude.as_ref())
            .is_some_and(ClaudeStatus::ready)
    })
}

#[gpui_kit::test]
fn first_launch_shows_setup_beside_the_editor(cx: &mut TestAppContext) {
    let h = open(cx, first_run(signed_out()));
    assert!(h.exists(cx, "setup-done"));
    assert!(h.exists(cx, "setup-run-login"));

    // The editor keeps the focus: typing goes into the document.
    h.type_text(cx, "Hello");
    assert_eq!(h.text(cx), "Hello");
    assert!(h.exists(cx, "setup-done"));
}

#[gpui_kit::test]
fn signing_in_from_the_wizard_runs_claude_and_notices_when_done(cx: &mut TestAppContext) {
    let h = open(cx, first_run(signed_out()));
    h.click(cx, "setup-run-login");
    assert_eq!(h.probe.ran(), ["/usr/local/bin/claude auth login"]);
    assert!(!claude_ready(&h, cx));

    // Signing in happens in Terminal; the wizard looks again by itself.
    h.probe.set(signed_in());
    h.wait(cx, crate::setup::RECHECK_EVERY.as_millis() as u64);
    assert!(claude_ready(&h, cx));

    // Once signed in, it stops looking.
    let checks = h.probe.checks();
    h.wait(cx, 10 * crate::setup::RECHECK_EVERY.as_millis() as u64);
    assert_eq!(h.probe.checks(), checks);

    h.click(cx, "setup-jev-skip");
    h.click(cx, "setup-done");
    assert!(!h.exists(cx, "setup-done"));
    assert!(h.read(cx, |ws, _| ws.settings.setup_done));
}

#[gpui_kit::test]
fn without_claude_code_it_offers_the_installer(cx: &mut TestAppContext) {
    let h = open(cx, first_run(ClaudeStatus::NotInstalled));
    h.click(cx, "setup-copy-install");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some(crate::setup::INSTALL_COMMAND)
    );
    h.click(cx, "setup-copy-login");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some(crate::setup::LOGIN_COMMAND)
    );
    h.click(cx, "setup-run-install");
    let ran = h.probe.ran();
    assert_eq!(ran.len(), 1);
    assert!(ran[0].starts_with(crate::setup::INSTALL_COMMAND));
    assert!(ran[0].ends_with("auth login"));
}

#[gpui_kit::test]
fn finishing_later_brings_it_back_next_launch_and_stops_checking(cx: &mut TestAppContext) {
    let h = open(cx, first_run(signed_out()));
    h.click(cx, "setup-done");
    assert!(!h.exists(cx, "setup-done"));
    assert!(!h.read(cx, |ws, _| ws.settings.setup_done));

    let checks = h.probe.checks();
    h.wait(cx, 10 * crate::setup::RECHECK_EVERY.as_millis() as u64);
    assert_eq!(h.probe.checks(), checks);
}

#[gpui_kit::test]
fn once_set_up_it_stays_closed_until_reopened(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    assert!(!h.exists(cx, "setup-done"));
    assert_eq!(h.probe.checks(), 0);

    h.update(cx, |ws, _, cx| ws.open_setup(cx));
    assert!(h.exists(cx, "setup-done"));
    assert!(claude_ready(&h, cx));
}

#[gpui_kit::test]
fn the_jev_step_saves_the_key_and_turns_jev_on(cx: &mut TestAppContext) {
    let h = open(cx, first_run(signed_in()));
    h.click(cx, "setup-copy-jev");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some(crate::setup::JEV_KEY_COMMAND)
    );

    h.click(cx, "setup-jev-use");
    assert!(h.read(cx, |ws, _| ws.key_input.is_some()));
    h.type_text(cx, "ts-key");
    h.click(cx, "key-save");
    assert_eq!(h.jev.stored_key().as_deref(), Some("ts-key"));
    assert!(h.read(cx, |ws, _| ws.settings.jev_prefilter));
    assert!(!h.exists(cx, "setup-jev-use"));
}
