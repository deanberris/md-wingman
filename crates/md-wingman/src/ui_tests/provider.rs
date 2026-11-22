//! Using another model provider: the panel in the Assistant pane, the
//! token in the Keychain, and restarting the assistant on the new provider.

use gpui_kit::TestAppContext;

use super::*;
use crate::provider::ProviderSettings;

fn open_panel(h: &Harness, cx: &mut TestAppContext) {
    h.update(cx, |ws, window, cx| ws.open_provider_panel(window, cx));
    assert!(h.exists(cx, "provider-save"));
}

fn set_token(h: &Harness, cx: &mut TestAppContext, token: &str) {
    let token = token.to_string();
    h.update(cx, move |ws, window, cx| {
        let input = ws.provider_panel.as_ref().unwrap().token.clone();
        input.update(cx, |s, cx| s.set_value(token, window, cx));
    });
}

fn label(h: &Harness, cx: &mut TestAppContext) -> String {
    h.read(cx, |ws, _| ws.service_status.label())
}

fn last_connection(h: &Harness) -> Connection {
    h.connections.lock().unwrap().last().cloned().unwrap()
}

fn openrouter() -> ProviderSettings {
    ProviderSettings {
        enabled: true,
        name: "OpenRouter".into(),
        base_url: "https://openrouter.ai/api".into(),
        model: "moonshotai/kimi-k2".into(),
        fast_model: String::new(),
        web_search: false,
    }
}

#[gpui_kit::test]
fn claude_is_the_default(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    assert_eq!(
        *h.connections.lock().unwrap(),
        [Connection::Claude(AuthMode::Subscription)]
    );
    // No provider, so the Keychain isn't touched.
    assert_eq!(h.tokens.reads(), 0);
}

#[gpui_kit::test]
fn switching_to_a_local_model_restarts_the_assistant_on_it(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    open_panel(&h, cx);
    h.click(cx, "provider-preset-ollama");
    // Picking a preset fills in its URL and moves on to the model.
    h.type_text(cx, "qwen3:14b");
    h.click(cx, "provider-save");

    assert!(!h.exists(cx, "provider-save"));
    assert_eq!(
        last_connection(&h),
        Connection::Provider {
            settings: ProviderSettings {
                enabled: true,
                name: "Ollama".into(),
                base_url: "http://localhost:11434".into(),
                model: "qwen3:14b".into(),
                fast_model: String::new(),
                web_search: false,
            },
            token: None,
        }
    );
    assert_eq!(label(&h, cx), "Assistant ready · Ollama · qwen3:14b");
    assert!(h.read(cx, |ws, _| ws.settings.provider.enabled));

    // Back to writing: the editor has the focus again.
    h.type_text(cx, "Hello");
    assert_eq!(h.text(cx), "Hello");
}

#[gpui_kit::test]
fn a_token_goes_to_the_keychain_and_to_the_service(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    open_panel(&h, cx);
    h.click(cx, "provider-preset-openrouter");
    h.type_text(cx, "moonshotai/kimi-k2");
    set_token(&h, cx, "sk-or-test");
    h.click(cx, "provider-save");

    assert_eq!(h.tokens.stored().as_deref(), Some("sk-or-test"));
    let Connection::Provider { settings, token } = last_connection(&h) else {
        panic!("expected a provider connection");
    };
    assert_eq!(token.as_deref(), Some("sk-or-test"));
    assert_eq!(settings.base_url, "https://openrouter.ai/api");
    // The token never goes in the settings file.
    let saved = serde_json::to_string(&h.read(cx, |ws, _| ws.settings.clone())).unwrap();
    assert!(!saved.contains("sk-or-test"));
}

#[gpui_kit::test]
fn at_launch_the_saved_provider_and_token_are_used(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            settings: Settings {
                provider: openrouter(),
                ..Default::default()
            },
            provider_token: Some("sk-or-saved"),
            ..Default::default()
        },
    );
    assert_eq!(h.tokens.reads(), 1);
    assert_eq!(
        last_connection(&h),
        Connection::Provider {
            settings: openrouter(),
            token: Some("sk-or-saved".into()),
        }
    );
    assert_eq!(
        label(&h, cx),
        "Assistant ready · OpenRouter · moonshotai/kimi-k2"
    );

    // Saving without typing a token keeps the saved one.
    open_panel(&h, cx);
    h.click(cx, "provider-save");
    assert_eq!(h.tokens.reads(), 2);
    let Connection::Provider { token, .. } = last_connection(&h) else {
        panic!("expected a provider connection");
    };
    assert_eq!(token.as_deref(), Some("sk-or-saved"));
}

#[gpui_kit::test]
fn going_back_to_claude_cancels_what_was_running(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            text: "Their are three towers in teh city.\n",
            settings: Settings {
                provider: openrouter(),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    h.select(cx, "Their are three towers in teh city.");
    h.press(cx, &shortcut("e"));
    assert_eq!(h.assistant.request_count(), 1);

    open_panel(&h, cx);
    h.click(cx, "provider-use-claude");
    assert!(h.assistant.was_cancelled(0));
    assert_eq!(
        h.read(cx, |ws, _| ws.run_cards()),
        vec![(TaskKind::CopyEdit, "failed: Cancelled".to_string())]
    );
    assert_eq!(
        last_connection(&h),
        Connection::Claude(AuthMode::Subscription)
    );
    assert_eq!(label(&h, cx), "Assistant ready · Claude subscription");
    // The provider's settings are kept for next time.
    let provider = h.read(cx, |ws, _| ws.settings.provider.clone());
    assert!(!provider.enabled);
    assert_eq!(provider.model, "moonshotai/kimi-k2");
}

#[gpui_kit::test]
fn incomplete_settings_are_not_used(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    open_panel(&h, cx);
    h.click(cx, "provider-preset-lmstudio");
    h.click(cx, "provider-save");
    assert!(h.exists(cx, "provider-save"));
    assert_eq!(h.connections.lock().unwrap().len(), 1);
    assert!(h.notice(cx).contains("model"), "{}", h.notice(cx));
}

#[gpui_kit::test]
fn the_setup_wizard_offers_a_provider_instead_of_signing_in(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            first_run: true,
            claude: ClaudeStatus::SignedOut {
                path: "/usr/local/bin/claude".into(),
            },
            ..Default::default()
        },
    );
    h.click(cx, "setup-provider");
    assert!(h.exists(cx, "provider-save"));
    h.click(cx, "provider-preset-ollama");
    h.type_text(cx, "qwen3");
    h.click(cx, "provider-save");

    // Step 1 is done without a Claude sign-in, and finishing sticks.
    h.click(cx, "setup-jev-skip");
    h.click(cx, "setup-done");
    assert!(!h.exists(cx, "setup-done"));
    assert!(h.read(cx, |ws, _| ws.settings.setup_done));
}

#[gpui_kit::test]
fn another_model_still_needs_claude_code_installed(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            first_run: true,
            claude: ClaudeStatus::NotInstalled,
            settings: Settings {
                provider: openrouter(),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    h.click(cx, "setup-run-install");
    assert_eq!(h.probe.ran(), [crate::setup::INSTALL_COMMAND]);
    h.click(cx, "setup-done");
    assert!(!h.read(cx, |ws, _| ws.settings.setup_done));
}
