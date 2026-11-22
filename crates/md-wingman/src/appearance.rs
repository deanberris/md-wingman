//! Light and dark mode, and the editor and preview fonts.

use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, ThemeMode, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::settings::Appearance;
use crate::workspace::Workspace;
use crate::{
    AppearanceAndFonts, DecreaseFontSize, IncreaseFontSize, ResetFontSize, ToggleDarkMode,
};

/// Monospace fonts worth offering, where installed.
const EDITOR_FONTS: &[&str] = &[
    "Menlo",
    "SF Mono",
    "Monaco",
    "JetBrains Mono",
    "Fira Code",
    "IBM Plex Mono",
    "Iosevka",
    "Source Code Pro",
    "Cascadia Code",
    "Berkeley Mono",
    "Courier New",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
];

/// Reading fonts for the preview, where installed.
const PREVIEW_FONTS: &[&str] = &[
    "Charter",
    "Iowan Old Style",
    "Georgia",
    "Palatino",
    "New York",
    "Source Serif 4",
    "IBM Plex Serif",
    "Helvetica Neue",
    "Avenir Next",
    "Inter",
    "DejaVu Serif",
    "DejaVu Sans",
];

static DEFAULT_MONO: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The fonts offered in the panel: the installed ones from each list.
#[derive(Default)]
pub struct FontChoices {
    editor: Vec<String>,
    preview: Vec<String>,
}

impl Workspace {
    /// Applies the appearance and fonts from the settings to the theme.
    pub(crate) fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dark = match self.settings.appearance {
            Appearance::System => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            Appearance::Light => false,
            Appearance::Dark => true,
        };
        if cx.theme().is_dark() != dark {
            Theme::change(
                if dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                Some(window),
                cx,
            );
        }
        // The theme's own monospace font, for "Default".
        let default_font = DEFAULT_MONO
            .get_or_init(|| cx.theme().mono_font_family.to_string())
            .clone();
        let font = self.settings.editor_font.clone().unwrap_or(default_font);
        let size = self.settings.editor_font_size;
        Theme::update(cx, |theme| {
            theme.mono_font_family = font.into();
            theme.mono_font_size = px(size);
        });
        cx.notify();
    }

    pub(crate) fn on_toggle_dark_mode(
        &mut self,
        _: &ToggleDarkMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let appearance = if cx.theme().is_dark() {
            Appearance::Light
        } else {
            Appearance::Dark
        };
        self.set_appearance(appearance, window, cx);
    }

    fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.appearance = appearance;
        self.save_settings(cx);
        self.apply_appearance(window, cx);
    }

    fn set_font_size(&mut self, size: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.editor_font_size = size.clamp(9., 32.);
        self.save_settings(cx);
        self.apply_appearance(window, cx);
        self.set_notice(
            format!("Editor font size {}", self.settings.editor_font_size),
            cx,
        );
    }

    pub(crate) fn on_increase_font_size(
        &mut self,
        _: &IncreaseFontSize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_font_size(self.settings.editor_font_size + 1., window, cx);
    }

    pub(crate) fn on_decrease_font_size(
        &mut self,
        _: &DecreaseFontSize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_font_size(self.settings.editor_font_size - 1., window, cx);
    }

    pub(crate) fn on_reset_font_size(
        &mut self,
        _: &ResetFontSize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let size = crate::settings::Settings::default().editor_font_size;
        self.set_font_size(size, window, cx);
    }

    pub(crate) fn on_appearance_and_fonts(
        &mut self,
        _: &AppearanceAndFonts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.font_choices.is_some() {
            self.font_choices = None;
        } else {
            let installed = window.text_system().all_font_names();
            let pick = |list: &[&str]| -> Vec<String> {
                list.iter()
                    .filter(|f| installed.iter().any(|i| i == *f))
                    .map(|f| f.to_string())
                    .collect()
            };
            self.font_choices = Some(FontChoices {
                editor: pick(EDITOR_FONTS),
                preview: pick(PREVIEW_FONTS),
            });
        }
        cx.notify();
    }

    pub(crate) fn render_appearance_panel(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let choices = self.font_choices.as_ref()?;
        let label = |text: &str| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(text.to_string())
        };

        let mut modes = h_flex().gap_1();
        for (i, (name, appearance)) in [
            ("System", Appearance::System),
            ("Light", Appearance::Light),
            ("Dark", Appearance::Dark),
        ]
        .into_iter()
        .enumerate()
        {
            let on = self.settings.appearance == appearance;
            modes = modes.child(
                Button::new(("appearance", i))
                    .label(name)
                    .xsmall()
                    .when(on, |b| b.primary())
                    .when(!on, |b| b.outline())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_appearance(appearance, window, cx)
                    })),
            );
        }

        let font_buttons = |id: &'static str,
                            fonts: &[String],
                            current: Option<&str>,
                            default_label: &str,
                            preview: bool,
                            cx: &mut Context<Self>| {
            let mut row = h_flex().flex_wrap().gap_1();
            let options = std::iter::once(None).chain(fonts.iter().map(|f| Some(f.clone())));
            for (i, font) in options.enumerate() {
                let on = current == font.as_deref();
                let mut button = Button::new((id, i))
                    .label(font.clone().unwrap_or_else(|| default_label.to_string()))
                    .xsmall()
                    .when(on, |b| b.primary())
                    .when(!on, |b| b.outline());
                let choice = font.clone();
                button = button.on_click(cx.listener(move |this, _, window, cx| {
                    if preview {
                        this.settings.preview_font = choice.clone();
                    } else {
                        this.settings.editor_font = choice.clone();
                    }
                    this.save_settings(cx);
                    this.apply_appearance(window, cx);
                }));
                row = row.child(button);
            }
            row
        };
        let editor_fonts = font_buttons(
            "editor-font",
            &choices.editor,
            self.settings.editor_font.as_deref(),
            "Default",
            false,
            cx,
        );
        let preview_fonts = font_buttons(
            "preview-font",
            &choices.preview,
            self.settings.preview_font.as_deref(),
            "System",
            true,
            cx,
        );
        let size = self.settings.editor_font_size;

        Some(
            v_flex()
                .gap_2()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Appearance and fonts"),
                        )
                        .child(
                            Button::new("appearance-close")
                                .label("×")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.font_choices = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(label("Mode"))
                .child(modes)
                .child(label("Editor font"))
                .child(editor_fonts)
                .child(label("Editor size"))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            Button::new("size-down")
                                .label("A−")
                                .xsmall()
                                .outline()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_font_size(size - 1., window, cx)
                                })),
                        )
                        .child(div().text_sm().px_2().child(format!("{size} pt")))
                        .child(
                            Button::new("size-up")
                                .label("A+")
                                .xsmall()
                                .outline()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_font_size(size + 1., window, cx)
                                })),
                        ),
                )
                .child(label("Preview font"))
                .child(preview_fonts)
                .into_any_element(),
        )
    }
}
