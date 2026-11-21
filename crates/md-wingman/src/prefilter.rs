//! Turning the Jev pre-filter on and off, and the panel for the TypeSafe API
//! key, which goes straight into the macOS Keychain.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Theme;
use gpui_kit::component::{h_flex, v_flex, Sizable as _};
use gpui_kit::*;

use crate::workspace::Workspace;
use crate::{SetJevKey, ToggleJevPrefilter};

impl Workspace {
    pub(crate) fn on_toggle_jev(
        &mut self,
        _: &ToggleJevPrefilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_jev_prefilter(!self.settings.jev_prefilter, window, cx);
    }

    pub(crate) fn set_jev_prefilter(
        &mut self,
        on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.jev_prefilter = on;
        self.save_settings(cx);
        // References join the checks; what's already written counts as done.
        self.continuous_baseline(cx);
        if on {
            if self.continuous.has_key(&*self.grader) {
                self.set_notice(
                    format!(
                        "Jev pre-filter on: acting on grades above {:.0}%",
                        self.settings.jev_threshold * 100.
                    ),
                    cx,
                );
            } else {
                self.open_key_panel(window, cx);
            }
        } else {
            self.set_notice("Jev pre-filter off", cx);
        }
        cx.notify();
    }

    pub(crate) fn on_set_jev_key(
        &mut self,
        _: &SetJevKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_key_panel(window, cx);
    }

    fn open_key_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("TypeSafe API key")
        });
        input.update(cx, |state, cx| state.focus(window, cx));
        self.key_input = Some(input);
        cx.notify();
    }

    fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.key_input.clone() else {
            return;
        };
        let key = input.read(cx).value().trim().to_string();
        if key.is_empty() {
            self.set_notice("Paste a TypeSafe API key first", cx);
            return;
        }
        match self.grader.store_key(&key) {
            Ok(()) => {
                self.continuous.key_changed(Some(key));
                self.key_input = None;
                self.set_notice("Saved the TypeSafe API key in your Keychain", cx);
                self.editor()
                    .update(cx, |state, cx| state.focus(window, cx));
            }
            Err(e) => self.set_notice(format!("Couldn't save the key in the Keychain: {e}"), cx),
        }
    }

    fn clear_key(&mut self, cx: &mut Context<Self>) {
        match self.grader.clear_key() {
            Ok(()) => {
                self.continuous.key_changed(None);
                self.key_input = None;
                self.set_notice("Removed the TypeSafe API key from your Keychain", cx);
            }
            Err(e) => self.set_notice(format!("Couldn't remove the key: {e}"), cx),
        }
    }

    pub(crate) fn render_key_panel(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let input = self.key_input.as_ref()?;
        Some(
            v_flex()
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
                        .child("TypeSafe API key for Jev"),
                )
                .child(div().text_xs().text_color(theme.muted_foreground).child(
                    "Jev grades each paragraph before continuous mode spends a Claude request \
                     on it. The key is kept in your macOS Keychain, not in the settings file.",
                ))
                .child(Input::new(input).mask_toggle())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("key-save")
                                .label("Save")
                                .xsmall()
                                .primary()
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save_key(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("key-clear")
                                .label("Remove key")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| this.clear_key(cx))),
                        )
                        .child(
                            Button::new("key-cancel")
                                .label("Cancel")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.key_input = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}
