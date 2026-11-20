//! The main window: the editor (or its preview) on the left, the assistant
//! sidebar on the right, and a thin status bar underneath.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{EditorState, InputEvent};
use gpui_kit::component::resizable::{
    h_resizable, resizable_panel, ResizablePanelEvent, ResizableState,
};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assistant::{auth_mode_from_env, Assistant, ServiceStatus};
use crate::document::{word_count, Document};
use crate::settings::Settings;
use crate::{NewFile, Open, Save, SaveAs, TogglePreview};

/// The narrowest and widest the assistant pane can be dragged to.
const SIDEBAR_MIN: f32 = 280.;
const EDITOR_MIN: f32 = 360.;
/// Comfortable line length for reading the preview.
const PREVIEW_MAX_WIDTH: f32 = 720.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Raw,
    Preview,
}

pub struct Workspace {
    focus_handle: FocusHandle,
    editor: Entity<EditorState>,
    document: Document,
    pub(crate) mode: ViewMode,
    /// Bumped on every edit, so the preview re-renders from fresh text.
    revision: usize,
    dirty: bool,
    pub(crate) assistant: Option<Assistant>,
    pub(crate) service_status: ServiceStatus,
    /// A message for the status bar, such as a save error.
    pub(crate) notice: Option<SharedString>,
    pub(crate) runs: Vec<crate::runs::TaskRun>,
    pub(crate) next_run_id: usize,
    pub(crate) settings: Settings,
    pub(crate) findings: Vec<crate::findings::Finding>,
    pub(crate) next_finding_id: u64,
    /// The fact-checks under the cursor at the last look, to re-render the
    /// sidebar only when they change.
    cursor_claims: Vec<u64>,
    /// The editor/assistant split.
    split: Entity<ResizableState>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .line_number(false)
                .folding(false)
                .indent_guides(false)
                .soft_wrap(true)
                .placeholder("Start writing…")
        });
        let split = cx.new(|_| ResizableState::default());
        let subscriptions = vec![
            cx.subscribe_in(&editor, window, Self::on_editor_event),
            // The editor doesn't announce cursor moves, so watch it for the
            // sidebar's "at the cursor" section.
            cx.observe(&editor, |this, _, cx| {
                let claims = this.claims_at_cursor(cx);
                if claims != this.cursor_claims {
                    this.cursor_claims = claims;
                    cx.notify();
                }
            }),
            cx.subscribe(&split, |this, split, _: &ResizablePanelEvent, cx| {
                // Remember the assistant pane's share of the width.
                let sizes = split.read(cx).sizes().clone();
                let total: f32 = sizes.iter().map(|s| f32::from(*s)).sum();
                if let (Some(last), true) = (sizes.last(), total > 0.) {
                    this.settings.assistant_fraction = f32::from(*last) / total;
                    this.save_settings(cx);
                }
            }),
        ];

        let (settings, settings_error) = Settings::load();
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            editor,
            document: Document::untitled(),
            mode: ViewMode::Raw,
            revision: 0,
            dirty: false,
            assistant: None,
            service_status: ServiceStatus::Starting,
            notice: None,
            runs: Vec::new(),
            next_run_id: 1,
            settings,
            findings: Vec::new(),
            next_finding_id: 1,
            cursor_claims: Vec::new(),
            split,
            _subscriptions: subscriptions,
        };
        if let Some(path) = path {
            this.open_path(path, window, cx);
        }
        if settings_error.is_some() {
            this.notice = settings_error.map(Into::into);
        }
        this.start_assistant(cx);
        this.editor.update(cx, |state, cx| state.focus(window, cx));
        this
    }

    fn start_assistant(&mut self, cx: &mut Context<Self>) {
        let auth = auth_mode_from_env();
        let start = cx
            .background_executor()
            .spawn(async move { Assistant::start(auth) });
        cx.spawn(async move |this, cx| {
            let result = start.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(assistant) => {
                        this.service_status = ServiceStatus::Ready(assistant.info.clone());
                        this.assistant = Some(assistant);
                    }
                    Err(message) => this.service_status = ServiceStatus::Failed(message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_editor_event(
        &mut self,
        _: &Entity<EditorState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::Change = event {
            self.revision += 1;
            let dirty = self.document.is_dirty(&self.text(cx));
            if dirty != self.dirty {
                self.dirty = dirty;
                window.set_window_title(&self.title());
            }
            self.reconcile_findings(cx);
            cx.notify();
        }
    }

    pub(crate) fn text(&self, cx: &App) -> String {
        self.editor.read(cx).value().to_string()
    }

    pub(crate) fn document_path(&self) -> Option<PathBuf> {
        self.document.path().map(PathBuf::from)
    }

    pub(crate) fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub fn title(&self) -> String {
        let dirty = if self.dirty { " •" } else { "" };
        format!("{}{dirty} — md-wingman", self.document.display_name())
    }

    pub(crate) fn set_notice(&mut self, notice: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.notice = Some(notice.into());
        cx.notify();
    }

    fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match Document::open(&path) {
            Ok((doc, text)) => {
                self.document = doc;
                self.editor
                    .update(cx, |state, cx| state.set_value(text, window, cx));
                self.dirty = false;
                self.revision += 1;
                self.notice = None;
                window.set_window_title(&self.title());
                self.load_claims(cx);
                cx.notify();
            }
            Err(e) => self.set_notice(format!("Couldn't open {}: {e}", path.display()), cx),
        }
    }

    fn save_to(&mut self, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let moved = path.is_some();
        match self.document.save(&text, path.as_deref()) {
            Ok(()) => {
                if moved {
                    // Fact-checks follow the document to its new name.
                    self.save_claims(cx);
                }
                self.dirty = false;
                self.notice = Some(format!("Saved {}", self.document.display_name()).into());
                window.set_window_title(&self.title());
                cx.notify();
            }
            Err(e) => self.set_notice(format!("Couldn't save: {e}"), cx),
        }
    }

    fn on_new(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.dirty {
            self.set_notice(
                "Save or discard your changes before starting a new document",
                cx,
            );
            return;
        }
        self.document = Document::untitled();
        self.editor
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.load_claims(cx);
        self.dirty = false;
        self.revision += 1;
        window.set_window_title(&self.title());
        cx.notify();
    }

    fn on_open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        if self.dirty {
            self.set_notice("Save your changes before opening another file", cx);
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.pop() else { return };
            let _ = this.update_in(cx, |this, window, cx| this.open_path(path, window, cx));
        })
        .detach();
    }

    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.document.path().is_some() {
            self.save_to(None, window, cx);
        } else {
            self.on_save_as(&SaveAs, window, cx);
        }
    }

    fn on_save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .document
            .path()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let name = match self.document.path() {
            Some(_) => self.document.display_name(),
            None => "Untitled.md".into(),
        };
        let path = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| this.save_to(Some(path), window, cx));
        })
        .detach();
    }

    fn on_toggle_preview(
        &mut self,
        _: &TogglePreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_mode(
            match self.mode {
                ViewMode::Raw => ViewMode::Preview,
                ViewMode::Preview => ViewMode::Raw,
            },
            window,
            cx,
        );
    }

    pub(crate) fn set_mode(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        match mode {
            ViewMode::Raw => self.editor.update(cx, |state, cx| state.focus(window, cx)),
            ViewMode::Preview => window.focus(&self.focus_handle, cx),
        }
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let name = self.document.display_name();
        h_flex()
            .h(px(40.))
            .px_4()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(if self.dirty {
                        format!("{name} •")
                    } else {
                        name
                    }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("mode-raw")
                            .label("Raw")
                            .small()
                            .when(self.mode == ViewMode::Raw, |b| b.primary())
                            .when(self.mode != ViewMode::Raw, |b| b.ghost())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_mode(ViewMode::Raw, window, cx)
                            })),
                    )
                    .child(
                        Button::new("mode-preview")
                            .label("Preview")
                            .small()
                            .when(self.mode == ViewMode::Preview, |b| b.primary())
                            .when(self.mode != ViewMode::Preview, |b| b.ghost())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_mode(ViewMode::Preview, window, cx)
                            })),
                    ),
            )
    }

    pub(crate) fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.settings.save() {
            self.set_notice(format!("Couldn't save settings: {e}"), cx);
        }
    }

    fn render_document(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match self.mode {
            ViewMode::Raw => self.render_raw_editor(window, cx),
            ViewMode::Preview => {
                let text = self.text(cx);
                div()
                    .id("preview-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .child(
                        h_flex().w_full().justify_center().child(
                            div()
                                .w_full()
                                .max_w(px(PREVIEW_MAX_WIDTH))
                                .px_8()
                                .py_8()
                                .child(
                                    TextView::markdown(
                                        ElementId::Name(
                                            format!("preview-{}", self.revision).into(),
                                        ),
                                        text,
                                    )
                                    .selectable(true),
                                ),
                        ),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let text = self.text(cx);
        let words = word_count(&text);
        let notice = self.notice.clone().unwrap_or_default();
        h_flex()
            .h(px(26.))
            .px_4()
            .gap_4()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .font_family(theme.mono_font_family.clone())
            .text_color(theme.muted_foreground)
            .child(format!("{words} words"))
            .child(div().flex_1().child(notice))
            .child(self.service_status.label())
    }

    pub(crate) fn render_sidebar_frame(
        &self,
        body: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                h_flex()
                    .h(px(40.))
                    .px_4()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .text_sm()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child("Assistant"),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_body = self.render_sidebar_body(cx);
        let (background, foreground) = (cx.theme().background, cx.theme().foreground);
        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_new))
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_save_as))
            .on_action(cx.listener(Self::on_toggle_preview))
            .on_action(cx.listener(Self::on_toggle_ruler))
            .on_action(cx.listener(Self::on_reflow))
            .on_action(cx.listener(Self::on_copy_edit))
            .on_action(cx.listener(Self::on_fact_check))
            .on_action(cx.listener(Self::on_find_references))
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child({
                // Initial sizes only; after that the split keeps what the
                // user dragged it to.
                let width = f32::from(window.viewport_size().width);
                let fraction = self.settings.assistant_fraction;
                div().flex_1().min_h_0().child(
                    h_resizable("split")
                        .with_state(&self.split)
                        .child(
                            resizable_panel()
                                .size(px(width * (1. - fraction)))
                                .size_range(px(EDITOR_MIN)..Pixels::MAX)
                                .child(
                                    v_flex().size_full().child(self.render_toolbar(cx)).child(
                                        div()
                                            .flex_1()
                                            .min_h_0()
                                            .child(self.render_document(window, cx)),
                                    ),
                                ),
                        )
                        .child(
                            resizable_panel()
                                .size(px(width * fraction))
                                .size_range(px(SIDEBAR_MIN)..Pixels::MAX)
                                .child(self.render_sidebar_frame(sidebar_body, cx)),
                        ),
                )
            })
            .child(self.render_status_bar(cx))
    }
}
