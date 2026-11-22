//! The Markdown quick reference: a panel in the Assistant pane, opened from
//! Help ▸ Markdown Guide, ⌘/ or the `?` button. It only ever opens when
//! asked, and never takes the focus from the editor.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, Theme, h_flex, v_flex};
use gpui_kit::*;

use crate::MarkdownGuide;
use crate::workspace::Workspace;

/// One line of the guide: what to type, what it makes, and the shortcut
/// that types it, if there is one.
pub struct Entry {
    pub syntax: &'static str,
    pub meaning: &'static str,
    pub keys: Option<&'static str>,
}

const fn entry(syntax: &'static str, meaning: &'static str, keys: Option<&'static str>) -> Entry {
    Entry {
        syntax,
        meaning,
        keys,
    }
}

/// The guide's sections, in order.
pub const SECTIONS: &[(&str, &[Entry])] = &[
    (
        "Text",
        &[
            entry("**bold**", "Bold", Some("B")),
            entry("_italic_", "Italic", Some("I")),
            entry("~~struck~~", "Strikethrough", Some("⇧X")),
            entry("`code`", "Code in a sentence", None),
            entry("line end\\", "Line break (or two spaces)", None),
        ],
    ),
    (
        "Links",
        &[
            entry("[text](https://…)", "Link", Some("K")),
            entry("<https://…>", "Link showing its address", None),
            entry("![alt text](image.png)", "Image", None),
            entry("claim[^1]", "Footnote mark", None),
            entry("[^1]: The note.", "Footnote, anywhere in the post", None),
        ],
    ),
    (
        "Blocks",
        &[
            entry("# Title", "Heading; ## and ### for smaller ones", None),
            entry("> quoted", "Quote", None),
            entry("- item", "Bulleted list (or * or +)", None),
            entry("1. item", "Numbered list", None),
            entry("- [ ] task", "Task list; [x] when done", None),
            entry("```", "Code block: a line of ``` before and after", None),
            entry("---", "Horizontal rule", None),
            entry("| a | b |", "Table row; |---|---| under the header", None),
        ],
    ),
];

impl Workspace {
    pub(crate) fn on_markdown_guide(
        &mut self,
        _: &MarkdownGuide,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_guide = !self.show_guide;
        cx.notify();
    }

    pub(crate) fn render_guide(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_guide {
            return None;
        }
        let mut panel = v_flex()
            .id("markdown-guide-panel")
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
                            .child("Markdown"),
                    )
                    .child(
                        Button::new("guide-close")
                            .label("×")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_guide = false;
                                cx.notify();
                            })),
                    ),
            );
        for (title, entries) in SECTIONS {
            panel = panel.child(
                div()
                    .pt_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(title.to_string()),
            );
            for e in entries.iter() {
                panel = panel.child(render_entry(e, theme));
            }
        }
        Some(
            panel
                .child(
                    div()
                        .pt_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(
                            "Select text and right-click, or use the Format menu, \
                             to add these for you. Leave a blank line between paragraphs.",
                        ),
                )
                .into_any_element(),
        )
    }
}

fn render_entry(e: &Entry, theme: &Theme) -> Div {
    h_flex()
        .gap_3()
        .items_start()
        .text_xs()
        .child(
            div()
                .w(px(150.))
                .flex_none()
                .font_family(theme.mono_font_family.clone())
                .child(e.syntax),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(theme.muted_foreground)
                .child(e.meaning),
        )
        .children(e.keys.map(|k| {
            div()
                .flex_none()
                .font_family(theme.mono_font_family.clone())
                .text_color(theme.muted_foreground)
                .child(shortcut(k))
        }))
}

/// A shortcut as the platform shows it, such as "⌘B".
fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        match key.strip_prefix('⇧') {
            Some(rest) => format!("⇧⌘{rest}"),
            None => format!("⌘{key}"),
        }
    } else {
        format!("Ctrl+{}", key.replace('⇧', "Shift+"))
    }
}
