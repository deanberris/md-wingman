//! Assistant tasks in the sidebar: starting them on the selected passage,
//! streaming their progress, and turning their replies into suggestions the
//! writer can accept, dismiss or jump to.

use std::ops::Range;

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{TextDecoration, TextDecorationCollection};
use gpui_kit::component::text::TextView;
use gpui_kit::component::Theme;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use wingman_client::{Event, TaskKind};

use crate::assistant::{task_title, ServiceStatus};
use crate::suggestions::{
    footnote_citation, link_citation, locate_all, passage_range, Claim, CopyEditReply,
    EditSuggestion, FactCheckReply, Insertion, Reference, ReferencesReply, Relation, Verdict,
};
use crate::workspace::{ViewMode, Workspace};
use crate::{CopyEdit, FactCheck, FindReferences};

/// Send the whole document as context only when it is reasonably small.
const MAX_CONTEXT_BYTES: usize = 40_000;

pub struct TaskRun {
    id: usize,
    kind: TaskKind,
    /// The first few words of the passage, to label the card.
    excerpt: String,
    /// Where the passage was when the task started.
    passage: Range<usize>,
    state: RunState,
    streamed: String,
    summary: String,
    items: Vec<RunItem>,
    cancel: Option<Box<dyn Fn() + Send>>,
    _events: Option<Task<()>>,
}

#[derive(Debug, Clone, PartialEq)]
enum RunState {
    Running(String),
    Done,
    Failed(String),
}

struct RunItem {
    kind: ItemKind,
    /// Tracks the item's text in the editor as the document changes.
    highlight: Option<TextDecorationCollection>,
    state: ItemState,
}

enum ItemKind {
    Edit(EditSuggestion),
    Claim(Claim),
    Reference(Reference),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemState {
    Open,
    Accepted,
    Dismissed,
    /// The text it refers to was changed or deleted.
    Stale,
}

/// An item before it is placed: what it is, the text it quotes, its style.
type RawItem = (ItemKind, String, HighlightStyle);

fn edit_style() -> HighlightStyle {
    HighlightStyle {
        underline: Some(UnderlineStyle {
            thickness: px(2.),
            color: Some(hsla(0.08, 0.9, 0.5, 1.)),
            wavy: true,
        }),
        ..Default::default()
    }
}

fn verdict_color(verdict: Verdict) -> Hsla {
    match verdict {
        Verdict::Supported => hsla(0.38, 0.6, 0.4, 1.),
        Verdict::Disputed => hsla(0.11, 0.9, 0.45, 1.),
        Verdict::False => hsla(0.0, 0.75, 0.5, 1.),
        Verdict::Unverifiable => hsla(0.0, 0.0, 0.5, 1.),
    }
}

fn relation_color(relation: Relation) -> Hsla {
    match relation {
        Relation::Supports => hsla(0.38, 0.6, 0.4, 1.),
        Relation::Challenges => hsla(0.0, 0.75, 0.5, 1.),
        Relation::Background => hsla(0.6, 0.5, 0.5, 1.),
    }
}

fn reference_style() -> HighlightStyle {
    HighlightStyle {
        underline: Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(hsla(0.6, 0.6, 0.5, 0.8)),
            wavy: false,
        }),
        ..Default::default()
    }
}

fn claim_style(verdict: Verdict) -> HighlightStyle {
    HighlightStyle {
        background_color: Some(verdict_color(verdict).opacity(0.18)),
        ..Default::default()
    }
}

impl Workspace {
    pub(crate) fn on_copy_edit(
        &mut self,
        _: &CopyEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_run(TaskKind::CopyEdit, window, cx);
    }

    pub(crate) fn on_find_references(
        &mut self,
        _: &FindReferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_run(TaskKind::FindReferences, window, cx);
    }

    pub(crate) fn on_fact_check(
        &mut self,
        _: &FactCheck,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_run(TaskKind::FactCheck, window, cx);
    }

    pub(crate) fn start_run(
        &mut self,
        kind: TaskKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.assistant.is_none() {
            self.set_notice("The assistant isn't running", cx);
            return;
        }
        if self.mode == ViewMode::Preview {
            self.set_mode(ViewMode::Raw, window, cx);
        }
        let text = self.text(cx);
        let selection = self.editor().read(cx).selected_range();
        let passage = passage_range(&text, selection);
        let passage_text = text[passage.clone()].to_string();
        if passage_text.trim().is_empty() {
            self.set_notice("Select some text, or put the cursor in a paragraph", cx);
            return;
        }
        let context = (text.len() <= MAX_CONTEXT_BYTES).then_some(text.as_str());
        let running =
            match self
                .assistant
                .as_ref()
                .expect("checked above")
                .run(kind, &passage_text, context)
            {
                Ok(r) => r,
                Err(e) => {
                    self.set_notice(e, cx);
                    return;
                }
            };

        let id = self.next_run_id;
        self.next_run_id += 1;
        let mut events = running.events;
        let task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let finished = matches!(event, Event::Done { .. } | Event::Error { .. });
                if this
                    .update(cx, |this, cx| this.on_run_event(id, event, cx))
                    .is_err()
                    || finished
                {
                    break;
                }
            }
        });
        let excerpt: String = passage_text
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ");
        self.runs.insert(
            0,
            TaskRun {
                id,
                kind,
                excerpt,
                passage,
                state: RunState::Running("Thinking…".into()),
                streamed: String::new(),
                summary: String::new(),
                items: Vec::new(),
                cancel: Some(running.cancel),
                _events: Some(task),
            },
        );
        cx.notify();
    }

    fn run_mut(&mut self, id: usize) -> Option<&mut TaskRun> {
        self.runs.iter_mut().find(|r| r.id == id)
    }

    fn on_run_event(&mut self, id: usize, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Delta { text } => {
                if let Some(run) = self.run_mut(id) {
                    run.streamed.push_str(&text);
                }
            }
            Event::Status { message } => {
                if let Some(run) = self.run_mut(id) {
                    run.state = RunState::Running(format!("{message}…"));
                }
            }
            Event::Done { text, data } => self.finish_run(id, text, data, cx),
            Event::Error { message } => {
                if let Some(run) = self.run_mut(id) {
                    run.cancel = None;
                    run.state = RunState::Failed(if message == "cancelled" {
                        "Cancelled".into()
                    } else {
                        message
                    });
                }
            }
            Event::Hello { .. } => {}
        }
        cx.notify();
    }

    fn finish_run(
        &mut self,
        id: usize,
        text: String,
        data: Option<serde_json::Value>,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.runs.iter().find(|r| r.id == id) else {
            return;
        };
        let (kind, passage) = (run.kind, run.passage.clone());
        let doc = self.text(cx);

        // Turn the reply into items, each with its text found in the document.
        let parsed: Result<(String, Vec<RawItem>), String> = match kind {
            TaskKind::CopyEdit => data
                .ok_or_else(|| "No suggestions came back".to_string())
                .and_then(|d| serde_json::from_value::<CopyEditReply>(d).map_err(|e| e.to_string()))
                .map(|r| {
                    let items = r
                        .edits
                        .into_iter()
                        .filter(|e| e.original != e.replacement)
                        .map(|e| {
                            let quote = e.original.clone();
                            (ItemKind::Edit(e), quote, edit_style())
                        })
                        .collect();
                    (r.summary, items)
                }),
            TaskKind::FactCheck => data
                .ok_or_else(|| "No claims came back".to_string())
                .and_then(|d| {
                    serde_json::from_value::<FactCheckReply>(d).map_err(|e| e.to_string())
                })
                .map(|r| {
                    let items = r
                        .claims
                        .into_iter()
                        .map(|c| {
                            let quote = c.quote.clone();
                            let style = claim_style(c.verdict);
                            (ItemKind::Claim(c), quote, style)
                        })
                        .collect();
                    (r.summary, items)
                }),
            TaskKind::FindReferences => data
                .ok_or_else(|| "No references came back".to_string())
                .and_then(|d| {
                    serde_json::from_value::<ReferencesReply>(d).map_err(|e| e.to_string())
                })
                .map(|r| {
                    let items = r
                        .references
                        .into_iter()
                        .map(|r| {
                            let quote = r.quote.clone();
                            (ItemKind::Reference(r), quote, reference_style())
                        })
                        .collect();
                    (r.summary, items)
                }),
        };

        let (summary, raw_items) = match parsed {
            Ok(v) => v,
            Err(e) => {
                if let Some(run) = self.run_mut(id) {
                    run.cancel = None;
                    run.state = RunState::Failed(format!("Couldn't read the reply: {e}"));
                }
                return;
            }
        };

        let ranges = if kind == TaskKind::FindReferences {
            // Several sources can back the same claim, so they may share a match.
            raw_items
                .iter()
                .map(|(_, q, _)| locate_all(&doc, [q.as_str()], &passage).pop().flatten())
                .collect()
        } else {
            locate_all(&doc, raw_items.iter().map(|(_, q, _)| q.as_str()), &passage)
        };
        // A reference whose claim can't be found is cited after the passage.
        let passage_intact = doc
            .get(passage.clone())
            .is_some_and(|p| !p.trim().is_empty());
        let mut items = Vec::new();
        for ((mut kind, _, mut style), mut range) in raw_items.into_iter().zip(ranges) {
            if let ItemKind::Reference(r) = &mut kind {
                if range.is_none() && passage_intact {
                    r.quote = doc[passage.clone()].to_string();
                    range = Some(passage.clone());
                    style = HighlightStyle::default();
                }
            }
            let highlight = range.map(|range| {
                self.editor().update(cx, |state, cx| {
                    state.create_decorations_collection(vec![TextDecoration::new(range, style)], cx)
                })
            });
            let state = if highlight.is_some() {
                ItemState::Open
            } else {
                ItemState::Stale
            };
            items.push(RunItem {
                kind,
                highlight,
                state,
            });
        }

        if let Some(run) = self.run_mut(id) {
            run.cancel = None;
            run.state = RunState::Done;
            run.summary = if summary.is_empty() { text } else { summary };
            run.items = items;
        }
    }

    /// The current range of an item's text, if it still matches what the
    /// assistant quoted.
    fn item_range(&self, run: usize, item: usize, cx: &App) -> Option<Range<usize>> {
        let run = self.runs.iter().find(|r| r.id == run)?;
        let item = run.items.get(item)?;
        let range = item.highlight.as_ref()?.get_ranges(cx).into_iter().next()?;
        let expected = match &item.kind {
            ItemKind::Edit(e) => &e.original,
            ItemKind::Claim(c) => &c.quote,
            ItemKind::Reference(r) => &r.quote,
        };
        let text = self.editor().read(cx).value();
        (text.get(range.clone()) == Some(expected.as_str())).then_some(range)
    }

    fn set_item_state(
        &mut self,
        run: usize,
        item: usize,
        state: ItemState,
        cx: &mut Context<Self>,
    ) {
        if let Some(it) = self.run_mut(run).and_then(|r| r.items.get_mut(item)) {
            it.state = state;
            if let Some(h) = it.highlight.take() {
                h.clear(cx);
            }
        }
        cx.notify();
    }

    fn accept_item(
        &mut self,
        run: usize,
        item: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = self.item_range(run, item, cx) else {
            self.set_item_state(run, item, ItemState::Stale, cx);
            return;
        };
        let replacement = match self
            .runs
            .iter()
            .find(|r| r.id == run)
            .and_then(|r| r.items.get(item))
            .map(|i| &i.kind)
        {
            Some(ItemKind::Edit(e)) => e.replacement.clone(),
            _ => return,
        };
        self.editor().update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.replace(replacement, window, cx);
        });
        self.set_item_state(run, item, ItemState::Accepted, cx);
    }

    fn accept_all(&mut self, run: usize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self
            .runs
            .iter()
            .find(|r| r.id == run)
            .map(|r| r.items.len())
            .unwrap_or(0);
        for i in 0..count {
            let open = self
                .runs
                .iter()
                .find(|r| r.id == run)
                .and_then(|r| r.items.get(i))
                .is_some_and(|it| it.state == ItemState::Open);
            if open {
                self.accept_item(run, i, window, cx);
            }
        }
    }

    fn reveal_item(
        &mut self,
        run: usize,
        item: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.item_range(run, item, cx) {
            Some(range) => {
                if self.mode == ViewMode::Preview {
                    self.set_mode(ViewMode::Raw, window, cx);
                }
                self.editor().update(cx, |state, cx| {
                    state.set_selected_range(range, cx);
                    state.focus(window, cx);
                });
            }
            None => self.set_item_state(run, item, ItemState::Stale, cx),
        }
    }

    /// Cites a reference after the claim it backs, as an inline link or a
    /// footnote.
    fn insert_reference(
        &mut self,
        run: usize,
        item: usize,
        footnote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = self.item_range(run, item, cx) else {
            self.set_item_state(run, item, ItemState::Stale, cx);
            return;
        };
        let Some(ItemKind::Reference(reference)) = self
            .runs
            .iter()
            .find(|r| r.id == run)
            .and_then(|r| r.items.get(item))
            .map(|i| &i.kind)
        else {
            return;
        };
        let text = self.text(cx);
        let (insertions, notice): (Vec<Insertion>, String) = if footnote {
            let (n, edits) = footnote_citation(&text, range.end, reference);
            (edits.into(), format!("Added footnote [^{n}]"))
        } else {
            (
                vec![link_citation(&text, range.end, reference)],
                "Added a link".into(),
            )
        };
        if self.mode == ViewMode::Preview {
            self.set_mode(ViewMode::Raw, window, cx);
        }
        self.editor().update(cx, |state, cx| {
            for ins in insertions {
                state.set_selected_range(ins.range, cx);
                state.replace(ins.text, window, cx);
            }
        });
        self.set_item_state(run, item, ItemState::Accepted, cx);
        self.set_notice(notice, cx);
    }

    fn close_run(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(pos) = self.runs.iter().position(|r| r.id == id) {
            let run = self.runs.remove(pos);
            if let Some(cancel) = &run.cancel {
                cancel();
            }
            for item in run.items {
                if let Some(h) = item.highlight {
                    h.clear(cx);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn render_sidebar_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let ready = matches!(self.service_status, ServiceStatus::Ready(_));

        let mut list = v_flex().gap_3().p_3();
        if self.runs.is_empty() {
            let message: SharedString = match &self.service_status {
                ServiceStatus::Starting => "Starting the assistant…".into(),
                ServiceStatus::Ready(_) => format!(
                    "Select a passage, or put the cursor in a paragraph, then choose an action above.\n\n{} copy-edit · {} fact-check · {} references",
                    shortcut("E"),
                    shortcut("⇧C"),
                    shortcut("⇧R"),
                )
                .into(),
                ServiceStatus::Failed(e) => e.clone().into(),
            };
            list = list.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(message),
            );
        }
        for run in &self.runs {
            list = list.child(self.render_run(run, &theme, cx));
        }

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("run-copy-edit")
                            .label("Copy-edit")
                            .small()
                            .outline()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_run(TaskKind::CopyEdit, window, cx)
                            })),
                    )
                    .child(
                        Button::new("run-fact-check")
                            .label("Fact-check")
                            .small()
                            .outline()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_run(TaskKind::FactCheck, window, cx)
                            })),
                    )
                    .child(
                        Button::new("run-references")
                            .label("References")
                            .small()
                            .outline()
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_run(TaskKind::FindReferences, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .id("runs-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
            .into_any_element()
    }

    fn render_run(&self, run: &TaskRun, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = run.id;
        let open_edits = run
            .items
            .iter()
            .filter(|i| i.state == ItemState::Open && matches!(i.kind, ItemKind::Edit(_)))
            .count();

        let header = h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(task_title(run.kind)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(format!("“{}…”", run.excerpt)),
            )
            .child(match &run.state {
                RunState::Running(_) => Button::new(("cancel", id))
                    .label("Cancel")
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(cancel) = this.run_mut(id).and_then(|r| r.cancel.take()) {
                            cancel();
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                _ => Button::new(("close", id))
                    .label("×")
                    .xsmall()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| this.close_run(id, cx)))
                    .into_any_element(),
            });

        let mut body = v_flex().gap_2();
        match &run.state {
            RunState::Running(status) => {
                body = body.child(
                    div()
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .child(status.clone()),
                );
                if !run.streamed.is_empty() {
                    body = body.child(
                        TextView::markdown(("stream", id), run.streamed.clone()).selectable(true),
                    );
                }
            }
            RunState::Failed(message) => {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(message.clone()),
                );
            }
            RunState::Done => {
                if !run.summary.is_empty() {
                    body = body.child(div().text_sm().child(run.summary.clone()));
                }
                if run.items.is_empty() {
                    body = body.child(div().text_sm().text_color(theme.muted_foreground).child(
                        match run.kind {
                            TaskKind::CopyEdit => "No changes suggested.",
                            TaskKind::FactCheck => "No checkable claims found.",
                            TaskKind::FindReferences => "No references found.",
                        },
                    ));
                }
                if open_edits > 1 {
                    body = body.child(
                        Button::new(("accept-all", id))
                            .label(format!("Accept all {open_edits}"))
                            .xsmall()
                            .primary()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.accept_all(id, window, cx)
                            })),
                    );
                }
                for (i, item) in run.items.iter().enumerate() {
                    body = body.child(self.render_item(id, i, item, theme, cx));
                }
            }
        }

        v_flex()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_item(
        &self,
        run: usize,
        index: usize,
        item: &RunItem,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let element_id = ElementId::NamedInteger(format!("item-{run}").into(), index as u64);
        let open = item.state == ItemState::Open;
        let state_note = match item.state {
            ItemState::Open => None,
            ItemState::Accepted if matches!(item.kind, ItemKind::Reference(_)) => Some("Inserted"),
            ItemState::Accepted => Some("Accepted"),
            ItemState::Dismissed => Some("Dismissed"),
            ItemState::Stale => Some("Text not found in the document"),
        };

        let content = match &item.kind {
            ItemKind::Edit(e) => v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .text_sm()
                        .font_family(theme.mono_font_family.clone())
                        .child(
                            div()
                                .line_through()
                                .text_color(theme.muted_foreground)
                                .child(e.original.clone()),
                        )
                        .child("→")
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(e.replacement.clone()),
                        ),
                )
                .when(!e.reason.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(e.reason.clone()),
                    )
                }),
            ItemKind::Claim(c) => {
                let mut sources = h_flex().flex_wrap().gap_2();
                for (si, source) in c.sources.iter().enumerate() {
                    let url = source.url.clone();
                    sources = sources.child(
                        div()
                            .id(ElementId::NamedInteger(
                                format!("src-{run}-{index}").into(),
                                si as u64,
                            ))
                            .text_xs()
                            .text_color(theme.link)
                            .underline()
                            .cursor_pointer()
                            .child(source.title.clone())
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                }
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(verdict_color(c.verdict))
                                    .border_1()
                                    .border_color(verdict_color(c.verdict).opacity(0.5))
                                    .child(c.verdict.label()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .italic()
                                    .child(format!("“{}”", c.quote)),
                            ),
                    )
                    .child(div().text_sm().child(c.explanation.clone()))
                    .when(!c.sources.is_empty(), |d| d.child(sources))
            }
            ItemKind::Reference(r) => {
                let url = r.url.clone();
                let meta: Vec<&str> = [r.authors.trim(), r.year.trim(), r.publisher.trim()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(
                                div()
                                    .flex_none()
                                    .px_1()
                                    .rounded_sm()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(relation_color(r.relation))
                                    .border_1()
                                    .border_color(relation_color(r.relation).opacity(0.5))
                                    .child(r.relation.label()),
                            )
                            .child(
                                div()
                                    .id(ElementId::NamedInteger(
                                        format!("ref-{run}").into(),
                                        index as u64,
                                    ))
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.link)
                                    .underline()
                                    .cursor_pointer()
                                    .child(r.title.clone())
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        cx.open_url(&url)
                                    }),
                            ),
                    )
                    .when(!meta.is_empty(), |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(meta.join(" · ")),
                        )
                    })
                    .when(!r.note.is_empty(), |d| {
                        d.child(div().text_sm().child(r.note.clone()))
                    })
            }
        };

        let is_edit = matches!(item.kind, ItemKind::Edit(_));
        let is_reference = matches!(item.kind, ItemKind::Reference(_));
        v_flex()
            .id(element_id)
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(theme.muted.opacity(0.4))
            .when(!open, |d| d.opacity(0.55))
            .when(open, |d| {
                d.cursor_pointer().hover(|s| s.bg(theme.muted)).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.reveal_item(run, index, window, cx)
                    }),
                )
            })
            .child(content)
            .when_some(state_note, |d, note| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(note),
                )
            })
            .when(open, |d| {
                d.child(
                    h_flex()
                        .gap_1()
                        .when(is_edit, |row| {
                            row.child(
                                Button::new(ElementId::NamedInteger(
                                    format!("accept-{run}").into(),
                                    index as u64,
                                ))
                                .label("Accept")
                                .xsmall()
                                .primary()
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.accept_item(run, index, window, cx)
                                    },
                                )),
                            )
                        })
                        .when(is_reference, |row| {
                            row.child(
                                Button::new(ElementId::NamedInteger(
                                    format!("footnote-{run}").into(),
                                    index as u64,
                                ))
                                .label("Insert footnote")
                                .xsmall()
                                .primary()
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.insert_reference(run, index, true, window, cx)
                                    },
                                )),
                            )
                            .child(
                                Button::new(ElementId::NamedInteger(
                                    format!("link-{run}").into(),
                                    index as u64,
                                ))
                                .label("Insert link")
                                .xsmall()
                                .outline()
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.insert_reference(run, index, false, window, cx)
                                    },
                                )),
                            )
                        })
                        .child(
                            Button::new(ElementId::NamedInteger(
                                format!("dismiss-{run}").into(),
                                index as u64,
                            ))
                            .label("Dismiss")
                            .xsmall()
                            .ghost()
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.set_item_state(run, index, ItemState::Dismissed, cx)
                                },
                            )),
                        ),
                )
            })
            .into_any_element()
    }
}

/// A shortcut hint for the empty sidebar, such as "⌘E".
fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{}", key.replace('⇧', "Shift+"))
    }
}
