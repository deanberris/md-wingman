//! Using another model through Claude Code: any endpoint that speaks
//! Anthropic's Messages API, such as OpenRouter or a model server on this
//! Mac (Ollama, LM Studio). Claude Code is pointed at it with
//! `ANTHROPIC_BASE_URL`; the service does that in its "provider" mode.
//!
//! The settings (name, URL, models) live in the settings file. The provider's
//! token, if it needs one, lives in the macOS Keychain, never in the settings
//! file. The panel for all of it opens in the Assistant pane from Assistant ▸
//! Model Provider…, and from the setup wizard. It never opens by itself.

use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::ModelProvider;
use crate::workspace::Workspace;

const KEYCHAIN_SERVICE: &str = "md-wingman";
const KEYCHAIN_ACCOUNT: &str = "model-provider-token";

/// Another Anthropic-compatible endpoint, from the settings file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProviderSettings {
    /// Use this instead of Claude.
    pub enabled: bool,
    /// Shown in the status bar, such as "OpenRouter".
    pub name: String,
    /// The base URL, without `/v1`, such as `https://openrouter.ai/api`.
    pub base_url: String,
    /// The model for the tasks, in the provider's naming.
    pub model: String,
    /// The model for Claude Code's quick background calls. Empty uses `model`.
    pub fast_model: String,
    /// The endpoint runs Anthropic's server-side web search tool. Most don't,
    /// so fact-checks and references then work from the model's own
    /// knowledge and pages it fetches.
    pub web_search: bool,
}

impl ProviderSettings {
    /// What's wrong with these settings, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        let url = self.base_url.trim();
        if url.is_empty() {
            return Some("Enter the provider's base URL");
        }
        if !(url.starts_with("https://") || url.starts_with("http://"))
            || url.len() <= "https://".len()
        {
            return Some("The base URL must start with https:// or http://");
        }
        if self.model.trim().is_empty() {
            return Some("Enter the model's name, as the provider spells it");
        }
        None
    }

    /// The name to show: the one given, or the URL's host.
    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        let url = self.base_url.trim();
        let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
        rest.split('/').next().unwrap_or(rest).to_string()
    }
}

/// A starting point for the panel's fields.
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    /// An example model; providers add and retire models all the time.
    pub model_hint: &'static str,
    pub web_search: bool,
    pub note: &'static str,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api",
        model_hint: "e.g. moonshotai/kimi-k2",
        web_search: false,
        note: "Many models from many labs. Paste an OpenRouter API key from \
               openrouter.ai/settings/keys. Model names look like vendor/model.",
    },
    Preset {
        id: "ollama",
        name: "Ollama",
        base_url: "http://localhost:11434",
        model_hint: "e.g. qwen3:14b",
        web_search: false,
        note: "Models on this Mac. Needs Ollama 0.14 or later, running, with the \
               model pulled (ollama pull <model>). No token needed.",
    },
    Preset {
        id: "lmstudio",
        name: "LM Studio",
        base_url: "http://localhost:1234",
        model_hint: "the model's identifier in LM Studio",
        web_search: false,
        note: "Models on this Mac. Start LM Studio's local server with the model \
               loaded. No token needed.",
    },
];

/// Where the provider's token is kept: the Keychain for real, a fake in
/// tests.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<String>, String>;
    fn store(&self, token: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

/// The token in the macOS Keychain.
pub struct KeychainTokens;

fn keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|e| e.to_string())
}

impl TokenStore for KeychainTokens {
    fn load(&self) -> Result<Option<String>, String> {
        match keychain()?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => {
                log::error!(target: "provider", "couldn't read the provider token from the Keychain: {e}");
                Err(e.to_string())
            }
        }
    }

    fn store(&self, token: &str) -> Result<(), String> {
        keychain()?
            .set_password(token)
            .map_err(|e| e.to_string())
            .inspect(|()| log::info!(target: "provider", "saved a provider token in the Keychain"))
    }

    fn clear(&self) -> Result<(), String> {
        match keychain()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {
                log::info!(target: "provider", "removed the provider token from the Keychain");
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

/// The panel's fields, while it's open.
pub struct ProviderPanel {
    pub name: Entity<InputState>,
    pub base_url: Entity<InputState>,
    pub model: Entity<InputState>,
    pub fast_model: Entity<InputState>,
    /// Empty keeps the saved token.
    pub token: Entity<InputState>,
    pub web_search: bool,
    /// The preset last picked, for its note.
    pub preset: Option<&'static str>,
}

impl Workspace {
    pub(crate) fn on_model_provider(
        &mut self,
        _: &ModelProvider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_provider_panel(window, cx);
    }

    /// Opens the panel with the current settings. Only ever from a menu or a
    /// button, so taking the focus for its first field is expected.
    pub(crate) fn open_provider_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.settings.provider.clone();
        let field =
            |value: &str, placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
                let (value, placeholder) = (value.to_string(), placeholder.to_string());
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .default_value(value)
                })
            };
        let name = field(&p.name, "Name, e.g. OpenRouter", window, cx);
        let base_url = field(
            &p.base_url,
            "Base URL, e.g. https://openrouter.ai/api",
            window,
            cx,
        );
        let model = field(&p.model, "Model", window, cx);
        let fast_model = field(
            &p.fast_model,
            "Fast model (optional; the model if empty)",
            window,
            cx,
        );
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("API key or token (empty keeps the saved one)")
        });
        base_url.update(cx, |state, cx| state.focus(window, cx));
        self.provider_panel = Some(ProviderPanel {
            name,
            base_url,
            model,
            fast_model,
            token,
            web_search: p.web_search,
            preset: None,
        });
        cx.notify();
    }

    fn apply_preset(
        &mut self,
        preset: &'static Preset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.provider_panel.as_mut() else {
            return;
        };
        panel.preset = Some(preset.id);
        panel.web_search = preset.web_search;
        let (name, base_url, model) = (
            panel.name.clone(),
            panel.base_url.clone(),
            panel.model.clone(),
        );
        name.update(cx, |s, cx| s.set_value(preset.name, window, cx));
        base_url.update(cx, |s, cx| s.set_value(preset.base_url, window, cx));
        model.update(cx, |s, cx| {
            s.set_placeholder(preset.model_hint, window, cx);
        });
        model.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    fn panel_settings(&self, cx: &App) -> Option<(ProviderSettings, String)> {
        let panel = self.provider_panel.as_ref()?;
        let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        Some((
            ProviderSettings {
                enabled: true,
                name: read(&panel.name),
                base_url: read(&panel.base_url).trim_end_matches('/').to_string(),
                model: read(&panel.model),
                fast_model: read(&panel.fast_model),
                web_search: panel.web_search,
            },
            read(&panel.token),
        ))
    }

    /// Saves the panel and switches the assistant over to the provider.
    pub(crate) fn save_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((provider, token)) = self.panel_settings(cx) else {
            return;
        };
        if let Some(problem) = provider.problem() {
            self.set_notice(problem, cx);
            return;
        }
        let token = if token.is_empty() {
            None
        } else {
            if let Err(e) = self.tokens.store(&token) {
                self.set_notice(format!("Couldn't save the token in the Keychain: {e}"), cx);
                return;
            }
            Some(token)
        };
        log::info!(
            target: "provider",
            "using {} at {}, model {}, fast model {}, web search {}",
            provider.display_name(),
            provider.base_url,
            provider.model,
            if provider.fast_model.is_empty() { "(the model)" } else { &provider.fast_model },
            provider.web_search
        );
        let name = provider.display_name();
        self.settings.provider = provider;
        self.save_settings(cx);
        self.provider_panel = None;
        self.restart_assistant(token, cx);
        self.set_notice(format!("Switching the assistant to {name}…"), cx);
        self.editor()
            .update(cx, |state, cx| state.focus(window, cx));
    }

    /// Goes back to Claude, keeping the provider's settings for next time.
    pub(crate) fn use_claude(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.provider_panel = None;
        if self.settings.provider.enabled {
            self.settings.provider.enabled = false;
            self.save_settings(cx);
            self.restart_assistant(None, cx);
            self.set_notice("Switching the assistant back to Claude…", cx);
        }
        self.editor()
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn clear_provider_token(&mut self, cx: &mut Context<Self>) {
        match self.tokens.clear() {
            Ok(()) => self.set_notice("Removed the provider's token from your Keychain", cx),
            Err(e) => self.set_notice(format!("Couldn't remove the token: {e}"), cx),
        }
    }

    pub(crate) fn render_provider_panel(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = self.provider_panel.as_ref()?;
        let note = |text: String| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let label = |text: &str| div().text_xs().child(text.to_string());
        let preset_note = panel
            .preset
            .and_then(|id| PRESETS.iter().find(|p| p.id == id))
            .map(|p| p.note);
        let mut presets = h_flex().gap_1().flex_wrap();
        for preset in PRESETS {
            presets = presets.child(
                Button::new(SharedString::from(format!("provider-preset-{}", preset.id)))
                    .label(preset.name)
                    .xsmall()
                    .when(panel.preset == Some(preset.id), |b| b.primary())
                    .when(panel.preset != Some(preset.id), |b| b.outline())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_preset(preset, window, cx)
                    })),
            );
        }
        let web_search = panel.web_search;
        let using = self.settings.provider.enabled;
        Some(
            v_flex()
                .id("provider-panel")
                .gap_2()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Model provider"),
                )
                .child(note(
                    "Use another model instead of Claude: any service that speaks Anthropic's \
                     API, such as OpenRouter, or a model running on this Mac. Claude Code still \
                     does the work, so suggestions look the same, but how good they are depends \
                     on the model. Your Claude sign-in is never sent to the provider."
                        .into(),
                ))
                .child(presets)
                .when_some(preset_note, |d, n| d.child(note(n.into())))
                .child(label("Name"))
                .child(Input::new(&panel.name))
                .child(label("Base URL"))
                .child(Input::new(&panel.base_url))
                .child(label("Model"))
                .child(Input::new(&panel.model))
                .child(label("Fast model"))
                .child(Input::new(&panel.fast_model))
                .child(label("API key or token"))
                .child(Input::new(&panel.token).mask_toggle())
                .child(note(
                    "Kept in your macOS Keychain, not in the settings file. Local servers \
                     don't need one."
                        .into(),
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Button::new("provider-web-search")
                                .label(if web_search {
                                    "Web search: on"
                                } else {
                                    "Web search: off"
                                })
                                .xsmall()
                                .when(web_search, |b| b.primary())
                                .when(!web_search, |b| b.outline())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(p) = this.provider_panel.as_mut() {
                                        p.web_search = !p.web_search;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            div().flex_1().min_w_0().child(note(
                                "Turn on only if the provider runs Anthropic's web search tool. \
                             Without it, fact-checks rely on what the model knows."
                                    .into(),
                            )),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("provider-save")
                                .label("Save and use")
                                .xsmall()
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_provider(window, cx)
                                })),
                        )
                        .child(
                            Button::new("provider-use-claude")
                                .label(if using {
                                    "Use Claude instead"
                                } else {
                                    "Cancel"
                                })
                                .xsmall()
                                .ghost()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.use_claude(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("provider-clear-token")
                                .label("Remove token")
                                .xsmall()
                                .ghost()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.clear_provider_token(cx)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderSettings;

    fn provider(base_url: &str, model: &str) -> ProviderSettings {
        ProviderSettings {
            enabled: true,
            base_url: base_url.into(),
            model: model.into(),
            ..Default::default()
        }
    }

    #[test]
    fn checks_the_url_and_model() {
        assert_eq!(provider("https://openrouter.ai/api", "x/y").problem(), None);
        assert_eq!(provider("http://localhost:11434", "qwen3").problem(), None);
        assert!(provider("", "m").problem().is_some());
        assert!(provider("localhost:11434", "m").problem().is_some());
        assert!(provider("https://", "m").problem().is_some());
        assert!(
            provider("https://openrouter.ai/api", " ")
                .problem()
                .is_some()
        );
    }

    #[test]
    fn names_itself_after_the_host_by_default() {
        assert_eq!(
            provider("http://localhost:11434/", "m").display_name(),
            "localhost:11434"
        );
        let named = ProviderSettings {
            name: "OpenRouter".into(),
            ..provider("https://openrouter.ai/api", "m")
        };
        assert_eq!(named.display_name(), "OpenRouter");
    }
}
