//! The preview: the document rendered as it will read.
//!
//! One Markdown view lives as long as the window and is given the document's
//! text whenever it changes, so places in it (a footnote, a block, a
//! finding's quote) can be addressed by where they are in the source, and
//! scrolled to.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::text::{MarkdownExtensions, RenderedText, TextView, TextViewState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::document::split_front_matter;
use crate::footnotes::{self, FootnotePlugin, Mark, ReferencePlugin};
use crate::workspace::{ViewMode, Workspace};

/// Comfortable line length for reading the preview.
const PREVIEW_MAX_WIDTH: f32 = 720.;

/// Where a place scrolled to ends up in the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Scroll just far enough to see it.
    Nearest,
    /// Put it near the top, as following a link does.
    Top,
}

/// The gap left above a place scrolled to the top.
const TOP_GAP: f32 = 24.;

pub struct Preview {
    state: Entity<TextViewState>,
    /// Scrolls the pane the view sits in.
    scroll: ScrollHandle,
    /// Where the body the view renders starts in the document: after the
    /// front matter, which is drawn apart from it.
    body_start: usize,
    /// A place in the document to scroll to once the view has parsed the
    /// text it is in.
    pending_reveal: Option<Range<usize>>,
    /// How the scroll under way lines its place up.
    align: Rc<Cell<Align>>,
    /// The footnote references and footnotes in the body, by where they
    /// are in it.
    footnotes: Vec<Mark>,
    _observe: Subscription,
}

impl Preview {
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        let state = cx.new(|cx| TextViewState::markdown("", cx).selectable(true));
        // A parse landing announces itself here, so a scroll that was
        // waiting for it can go ahead.
        let observe = cx.observe(&state, |this, _, cx| this.flush_preview_reveal(cx));
        Preview {
            state,
            scroll: ScrollHandle::new(),
            body_start: 0,
            pending_reveal: None,
            align: Rc::new(Cell::new(Align::Nearest)),
            footnotes: Vec::new(),
            _observe: observe,
        }
    }

    /// The preview's rendered text, when it is of `body`. Just after an
    /// edit it may still be of the text before it.
    fn rendered(&self, body: &str, cx: &App) -> Option<RenderedText> {
        let rendered = self.state.read(cx).rendered_text();
        (rendered.source() == body).then_some(rendered)
    }

    /// How many places cite the footnote labelled `label`.
    pub(crate) fn citation_count(&self, label: &str) -> usize {
        footnotes::citations(&self.footnotes, label).count()
    }

    /// Which of the citations of `label` starts at `start` in the body.
    pub(crate) fn citation_index(&self, label: &str, start: usize) -> usize {
        footnotes::citations(&self.footnotes, label)
            .position(|range| range.start == start)
            .unwrap_or(0)
    }
}

impl Workspace {
    /// Gives the preview the document's text as it is now. Cheap when
    /// nothing changed.
    pub(crate) fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let (_, body) = split_front_matter(&text);
        self.preview.body_start = text.len() - body.len();
        self.preview.footnotes = footnotes::marks(body);
        self.preview
            .state
            .update(cx, |state, cx| state.set_text(body, cx));
    }

    /// Where a range of the document (in bytes of its Markdown) is in the
    /// preview's rendered text. `None` when nothing of it is rendered, such
    /// as front matter or a heading's `#`, or when the preview hasn't parsed
    /// the current text yet.
    pub(crate) fn preview_range(&self, source: Range<usize>, cx: &App) -> Option<Range<usize>> {
        let text = self.text(cx);
        let start = self.preview.body_start;
        let body = text.get(start..)?;
        let source = source.start.checked_sub(start)?..source.end.checked_sub(start)?;
        self.preview.rendered(body, cx)?.range_for_source(source)
    }

    /// Scrolls the preview to a range of the document, when the preview is
    /// showing. If the preview is still parsing an edit, it scrolls once
    /// that lands.
    pub(crate) fn reveal_in_preview(
        &mut self,
        source: Range<usize>,
        align: Align,
        cx: &mut Context<Self>,
    ) {
        if self.mode != ViewMode::Preview {
            return;
        }
        self.preview.align.set(align);
        self.sync_preview(cx);
        if self.starts_the_body(&source, cx) {
            // The first thing in the document: show what's above it too
            // (the front matter and the top margin).
            self.preview.pending_reveal = None;
            self.preview.scroll.set_offset(Point::default());
            cx.notify();
            return;
        }
        self.preview.pending_reveal = Some(source);
        self.flush_preview_reveal(cx);
    }

    /// Whether nothing is rendered above `source`: it is in the front
    /// matter, or on the first line of text after it.
    fn starts_the_body(&self, source: &Range<usize>, cx: &App) -> bool {
        let text = self.text(cx);
        let start = self.preview.body_start;
        let Some(body) = text.get(start..) else {
            return false;
        };
        let first = start + (body.len() - body.trim_start().len());
        source.start <= first
            || text
                .get(first..source.start)
                .is_some_and(|before| !before.contains('\n'))
    }

    /// Scrolls the preview to the footnote labelled `label`.
    pub(crate) fn go_to_footnote(&mut self, label: &str, cx: &mut Context<Self>) {
        let Some(range) = footnotes::definition(&self.preview.footnotes, label) else {
            return;
        };
        // The view can't scroll to a block a plugin draws, so scroll the
        // end of the text before the footnote to the top: the footnote is
        // then the next thing down.
        let text = self.text(cx);
        let start = self.preview.body_start;
        let before = text[start..start + range.start].trim_end();
        let last = before.chars().next_back().map_or(0, char::len_utf8);
        let end = start + before.len();
        self.reveal_in_preview(end - last..end, Align::Top, cx);
    }

    /// Scrolls the preview to the `nth` place that cites the footnote
    /// labelled `label`.
    pub(crate) fn go_to_citation(&mut self, label: &str, nth: usize, cx: &mut Context<Self>) {
        let Some(range) = footnotes::citations(&self.preview.footnotes, label).nth(nth) else {
            return;
        };
        let start = self.preview.body_start;
        self.reveal_in_preview(start + range.start..start + range.end, Align::Top, cx);
    }

    fn flush_preview_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.preview.pending_reveal.clone() else {
            return;
        };
        let text = self.text(cx);
        let Some(body) = text.get(self.preview.body_start..) else {
            return;
        };
        if self.preview.rendered(body, cx).is_none() {
            // Still parsing: the view's next change tries again.
            return;
        }
        self.preview.pending_reveal = None;
        let Some(range) = self.preview_range(source, cx) else {
            return;
        };
        self.preview.state.update(cx, |state, cx| {
            if let Err(e) = state.reveal_range(range, cx) {
                log::warn!("couldn't scroll the preview: {e:?}");
            }
        });
    }

    /// How far down the preview is scrolled, for tests.
    #[cfg(test)]
    pub(crate) fn preview_scrolled(&self) -> Pixels {
        -self.preview.scroll.offset().y
    }

    /// The preview's text as rendered, for tests.
    #[cfg(test)]
    pub(crate) fn preview_text(&self, cx: &App) -> String {
        let rendered = self.preview.state.read(cx).rendered_text();
        rendered.as_str().to_string()
    }

    pub(crate) fn render_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let text = self.text(cx);
        let (front_matter, _) = split_front_matter(&text);
        let front_matter = front_matter
            .filter(|f| !f.trim().is_empty())
            .map(|f| render_front_matter(f, cx));
        let scroll = self.preview.scroll.clone();
        let align = self.preview.align.clone();
        let extensions = MarkdownExtensions::default()
            .plugin(ReferencePlugin(cx.weak_entity()))
            .plugin(FootnotePlugin(cx.weak_entity()));
        let view = TextView::new(&self.preview.state)
            .markdown_extensions(extensions)
            .on_reveal(move |line, _, _| {
                // The pane is a plain scrolling div, so bring the line into it
                // here.
                let viewport = scroll.bounds();
                let mut offset = scroll.offset();
                let top = viewport.top() + px(TOP_GAP);
                if align.get() == Align::Top {
                    offset.y += top - line.top();
                } else if line.top() < viewport.top() {
                    offset.y += viewport.top() - line.top();
                } else if line.bottom() > viewport.bottom() {
                    offset.y -= line.bottom() - viewport.bottom();
                }
                // No further than the pane scrolls.
                offset.y = offset.y.clamp(-scroll.max_offset().y, px(0.));
                scroll.set_offset(offset);
            });
        div()
            .id("preview-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.preview.scroll)
            .child(
                h_flex().w_full().justify_center().child(
                    div()
                        .w_full()
                        .max_w(px(PREVIEW_MAX_WIDTH))
                        .px_8()
                        .py_8()
                        .when_some(self.settings.preview_font.clone(), |d, font| {
                            d.font_family(font)
                        })
                        .children(front_matter)
                        .child(view),
                ),
            )
            .into_any_element()
    }
}

/// Front matter in the preview: a small, muted block of its lines, so the
/// post's metadata is visible without reading as part of the post.
fn render_front_matter(front_matter: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    v_flex()
        .id("preview-front-matter")
        .test_support()
        .mb_6()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(theme.muted)
        .text_xs()
        .text_color(theme.muted_foreground)
        .font_family(theme.mono_font_family.clone())
        .children(
            front_matter
                .lines()
                .map(|line| div().child(SharedString::from(line.to_string()))),
        )
        .into_any_element()
}
