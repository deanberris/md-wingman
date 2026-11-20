//! The ruler: a vertical guide at the configured column, soft-wrapping at it,
//! and hard-wrapping paragraphs to it.

use gpui_kit::component::input::Editor;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::reflow::reflow;
use crate::suggestions::passage_range;
use crate::workspace::{ViewMode, Workspace};
use crate::{Reflow, ToggleRuler};

/// Horizontal space around the editor's text area.
const EDITOR_PADDING_X: f32 = 24.;
/// The editor's own inset between its edge and the first column.
const EDITOR_INSET_X: f32 = 12.;

impl Workspace {
    pub(crate) fn on_toggle_ruler(
        &mut self,
        _: &ToggleRuler,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.show_ruler = !self.settings.show_ruler;
        self.save_settings(cx);
        cx.notify();
    }

    /// Hard-wraps the selection, or the paragraph under the cursor, to the
    /// ruler column, as one undoable change.
    pub(crate) fn on_reflow(&mut self, _: &Reflow, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == ViewMode::Preview {
            self.set_mode(ViewMode::Raw, window, cx);
        }
        let text = self.text(cx);
        let selection = self.editor().read(cx).selected_range();
        let range = passage_range(&text, selection);
        let Some(wrapped) = reflow(&text[range.clone()], self.settings.ruler_column) else {
            self.set_notice("Already wrapped", cx);
            return;
        };
        let end = range.start + wrapped.len();
        self.editor().update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.replace(wrapped, window, cx);
            state.set_selected_range(end..end, cx);
        });
        self.set_notice(
            format!("Reflowed to {} columns", self.settings.ruler_column),
            cx,
        );
    }

    /// The width of one column of the editor's monospace font.
    fn column_width(&self, window: &Window, cx: &App) -> Option<Pixels> {
        let theme = cx.theme();
        let text = window.text_system();
        let font_id = text.resolve_font(&font(theme.mono_font_family.clone()));
        text.advance(font_id, theme.mono_font_size, 'm')
            .ok()
            .map(|size| size.width)
    }

    /// The raw editor, wrapping at the ruler column and drawing the ruler.
    pub(crate) fn render_raw_editor(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let column = self.settings.ruler_column as f32;
        let ruler_x = self
            .column_width(window, cx)
            .map(|w| px(EDITOR_INSET_X) + w * column);
        let border = cx.theme().border;

        div()
            .size_full()
            .px(px(EDITOR_PADDING_X))
            .py_4()
            .child(
                div()
                    .relative()
                    .h_full()
                    // Wrapping at the ruler: the editor is just wide enough
                    // for the column, plus room for the caret.
                    .when_some(ruler_x.filter(|_| self.settings.wrap_at_ruler), |d, x| {
                        // Both insets, plus slack for the caret and scrollbar.
                        d.max_w(x + px(EDITOR_INSET_X + 16.))
                    })
                    .child(
                        Editor::new(self.editor())
                            .appearance(false)
                            .bordered(false)
                            .h_full(),
                    )
                    .when_some(ruler_x.filter(|_| self.settings.show_ruler), |d, x| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(x)
                                .w(px(1.))
                                .bg(border.opacity(0.7)),
                        )
                    }),
            )
            .into_any_element()
    }
}
