//! Assistant tasks in the sidebar: starting them on the selected passage,
//! streaming their progress, and turning their replies into suggestions the
//! writer can accept, dismiss or jump to.

use std::ops::Range;

use futures::StreamExt as _;
use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use wingman_client::{Event, TaskKind};

use crate::assistant::{ServiceStatus, task_title};
use crate::continuous::{CheckStatus, Verdict, verdicts};
use crate::findings::{Finding, FindingKind, FindingState, relation_color, verdict_color};
use crate::suggestions::{
    CopyEditReply, FactCheckReply, ReferencesReply, locate_all, passage_range,
};
use crate::workspace::{ViewMode, Workspace};
use crate::{CopyEdit, FactCheck, FindReferences};

/// Send the whole document as context only when it is reasonably small.
pub(crate) const MAX_CONTEXT_BYTES: usize = 40_000;

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
    /// The findings this run produced.
    items: Vec<u64>,
    /// Started by continuous mode: it has no card, and its findings go
    /// straight to the document's lists.
    background: bool,
    /// Part of a review of the whole document (and so also `background`).
    review: bool,
    cancel: Option<Box<dyn Fn() + Send>>,
    _events: Option<Task<()>>,
    /// When it started, for the log.
    started: std::time::Instant,
}

#[derive(Debug, Clone, PartialEq)]
enum RunState {
    Running(String),
    Done,
    Failed(String),
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
        if text[passage.clone()].trim().is_empty() {
            self.set_notice("Select some text, or put the cursor in a paragraph", cx);
            return;
        }
        self.start_run_on(kind, passage, false, cx);
    }

    /// Starts a task on a passage, and returns its run's id.
    pub(crate) fn start_run_on(
        &mut self,
        kind: TaskKind,
        passage: Range<usize>,
        background: bool,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let text = self.text(cx);
        let context = (text.len() <= MAX_CONTEXT_BYTES).then(|| text.clone());
        self.start_run_with(kind, passage, background, context, cx)
    }

    /// Starts a task on a passage with the given context, and returns its
    /// run's id.
    pub(crate) fn start_run_with(
        &mut self,
        kind: TaskKind,
        passage: Range<usize>,
        background: bool,
        context: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let text = self.text(cx);
        let passage_text = text.get(passage.clone())?.to_string();
        let running = match self
            .assistant
            .as_ref()?
            .run(kind, &passage_text, context.as_deref())
        {
            Ok(r) => r,
            Err(e) => {
                log::error!(target: "runs", "couldn't start a {kind:?} run: {e}");
                self.set_notice(e, cx);
                return None;
            }
        };

        let id = self.next_run_id;
        self.next_run_id += 1;
        log::info!(
            target: "runs",
            "run {id}: {kind:?} {} started on {}, context {} bytes",
            if background { "in the background" } else { "from the menu" },
            wingman_log::text(&passage_text),
            context.as_ref().map_or(0, String::len)
        );
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
                background,
                review: false,
                cancel: Some(running.cancel),
                _events: Some(task),
                started: std::time::Instant::now(),
            },
        );
        cx.notify();
        Some(id)
    }

    pub(crate) fn mark_review_run(&mut self, id: usize) {
        if let Some(run) = self.run_mut(id) {
            run.review = true;
        }
    }

    pub(crate) fn run_passage(&self, id: usize) -> Option<Range<usize>> {
        self.runs
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.passage.clone())
    }

    pub(crate) fn set_run_passage(&mut self, id: usize, passage: Range<usize>) {
        if let Some(run) = self.run_mut(id) {
            run.passage = passage;
        }
    }

    pub(crate) fn cancel_run(&mut self, id: usize, cx: &mut Context<Self>) {
        log::info!(target: "runs", "run {id}: cancelling");
        if let Some(cancel) = self.run_mut(id).and_then(|r| r.cancel.take()) {
            cancel();
        }
        cx.notify();
    }

    /// A background run is done with: its findings join the document's
    /// lists, and continuous mode moves on.
    fn end_background_run(&mut self, id: usize, failure: Option<String>, cx: &mut Context<Self>) {
        let Some(pos) = self.runs.iter().position(|r| r.id == id) else {
            return;
        };
        let run = self.runs.remove(pos);
        for fid in &run.items {
            if let Some(f) = self.finding_mut(*fid) {
                f.run = None;
            }
        }
        if run.review {
            self.review_run_ended(id, failure, cx);
        } else {
            self.continuous_run_ended(id, run.kind, failure, run.items.len(), cx);
        }
    }

    /// Each card's kind and state, top first, such as "done: 2 suggested
    /// edits" or "failed: Cancelled". Background runs are left out.
    #[cfg(test)]
    pub(crate) fn run_cards(&self) -> Vec<(TaskKind, String)> {
        self.runs
            .iter()
            .filter(|r| !r.background)
            .map(|r| {
                let state = match &r.state {
                    RunState::Running(s) => format!("running: {s}"),
                    RunState::Done => format!("done: {}", r.summary),
                    RunState::Failed(m) => format!("failed: {m}"),
                };
                (r.kind, state)
            })
            .collect()
    }

    /// The ids of the runs, top first, for element ids such as `close`.
    #[cfg(test)]
    pub(crate) fn run_ids(&self) -> Vec<usize> {
        self.runs.iter().map(|r| r.id).collect()
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
                log::debug!(target: "runs", "run {id}: {message}");
                if let Some(run) = self.run_mut(id) {
                    run.state = RunState::Running(format!("{message}…"));
                }
            }
            Event::Done { text, data } => {
                let failure = self.finish_run(id, text, data, cx);
                if let Some(run) = self.run_mut(id) {
                    match &failure {
                        None => log::info!(
                            target: "runs",
                            "run {id}: {:?} done in {:?}, {} findings",
                            run.kind,
                            run.started.elapsed(),
                            run.items.len()
                        ),
                        Some(e) => log::error!(target: "runs", "run {id}: {e}"),
                    }
                }
                if self.run_mut(id).is_some_and(|r| r.background) {
                    self.end_background_run(id, failure, cx);
                }
            }
            Event::Error { message } => {
                if let Some(run) = self.run_mut(id) {
                    let elapsed = run.started.elapsed();
                    if message == "cancelled" {
                        log::info!(target: "runs", "run {id}: cancelled after {elapsed:?}");
                    } else {
                        log::warn!(target: "runs", "run {id}: {:?} failed after {elapsed:?}: {message}", run.kind);
                    }
                }
                if self.run_mut(id).is_some_and(|r| r.background) {
                    self.end_background_run(id, Some(message), cx);
                } else if let Some(run) = self.run_mut(id) {
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
    ) -> Option<String> {
        let run = self.runs.iter().find(|r| r.id == id)?;
        let (kind, passage, background) = (run.kind, run.passage.clone(), run.background);
        let doc = self.text(cx);

        // Turn the reply into findings, each with its text found in the document.
        type Parsed = (String, Vec<(FindingKind, String)>);
        let parsed: Result<Parsed, String> = match kind {
            TaskKind::CopyEdit => data
                .ok_or_else(|| "No suggestions came back".to_string())
                .and_then(|d| serde_json::from_value::<CopyEditReply>(d).map_err(|e| e.to_string()))
                .map(|r| {
                    let items = r
                        .edits
                        .into_iter()
                        .filter(|e| e.original != e.replacement)
                        .map(|e| {
                            let q = e.original.clone();
                            (FindingKind::Edit(e), q)
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
                            let q = c.quote.clone();
                            (FindingKind::Claim(c), q)
                        })
                        .collect();
                    (r.summary, items)
                }),
            TaskKind::FindReferences => data
                .ok_or_else(|| "No references came back".to_string())
                .and_then(|d| {
                    serde_json::from_value::<ReferencesReply>(d).map_err(|e| e.to_string())
                })
                .map(|mut r| {
                    r.dedupe();
                    let items = r
                        .references
                        .into_iter()
                        .map(|r| {
                            let q = r.quote.clone();
                            (FindingKind::Reference(r), q)
                        })
                        .collect();
                    (r.summary, items)
                }),
        };

        let (summary, raw_items) = match parsed {
            Ok(v) => v,
            Err(e) => {
                let message = format!("Couldn't read the reply: {e}");
                if let Some(run) = self.run_mut(id) {
                    run.cancel = None;
                    run.state = RunState::Failed(message.clone());
                }
                return Some(message);
            }
        };

        // A new check of a paragraph as you write replaces the suggestions
        // from its last check.
        if background && kind != TaskKind::FactCheck {
            let old: Vec<u64> = self
                .findings
                .iter()
                .filter(|f| {
                    let same_kind = match f.kind {
                        FindingKind::Edit(_) => kind == TaskKind::CopyEdit,
                        FindingKind::Reference(_) => kind == TaskKind::FindReferences,
                        FindingKind::Claim(_) => false,
                    };
                    same_kind
                        && f.run.is_none()
                        && f.state == FindingState::Open
                        && passage.contains(&f.anchor)
                })
                .map(|f| f.id)
                .collect();
            self.drop_findings(&old, cx);
        }

        // A new fact-check of a passage replaces the old results for it.
        if kind == TaskKind::FactCheck {
            let old: Vec<u64> = self
                .findings
                .iter()
                .filter(|f| f.is_claim() && passage.contains(&f.anchor))
                .map(|f| f.id)
                .collect();
            self.drop_findings(&old, cx);
        }

        let ranges: Vec<Option<Range<usize>>> = if kind == TaskKind::FindReferences {
            // Several sources can back the same claim, so they may share a match.
            raw_items
                .iter()
                .map(|(_, q)| locate_all(&doc, [q.as_str()], &passage).pop().flatten())
                .collect()
        } else {
            locate_all(&doc, raw_items.iter().map(|(_, q)| q.as_str()), &passage)
        };
        // A reference whose claim can't be found is cited after the passage.
        let passage_text = doc.get(passage.clone()).map(str::to_string);
        let mut items = Vec::new();
        for ((mut kind, quote), range) in raw_items.into_iter().zip(ranges) {
            let (quote, anchor, quiet) = match (range, &mut kind, &passage_text) {
                (Some(r), _, _) => (quote, r.start, false),
                (None, FindingKind::Reference(r), Some(p)) if !p.trim().is_empty() => {
                    r.quote = p.clone();
                    (p.clone(), passage.start, true)
                }
                (None, _, _) => (quote, passage.start, false),
            };
            items.push(self.add_finding(kind, quote, anchor, Some(id), quiet));
        }
        self.reconcile_findings(cx);
        if kind == TaskKind::FactCheck {
            self.save_claims(cx);
        }

        if let Some(run) = self.run_mut(id) {
            run.cancel = None;
            run.state = RunState::Done;
            run.summary = if summary.is_empty() { text } else { summary };
            run.items = items;
        }
        None
    }

    /// Closes a run's card. Its fact-checks stay with the text; its other
    /// findings go.
    fn close_run(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(pos) = self.runs.iter().position(|r| r.id == id) {
            let run = self.runs.remove(pos);
            if let Some(cancel) = &run.cancel {
                cancel();
            }
            let mut drop = Vec::new();
            for fid in run.items {
                match self.findings.iter_mut().find(|f| f.id == fid) {
                    Some(f) if f.is_claim() => f.run = None,
                    Some(_) => drop.push(fid),
                    None => {}
                }
            }
            self.drop_findings(&drop, cx);
        }
        cx.notify();
    }

    pub(crate) fn render_sidebar_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let ready = matches!(self.service_status, ServiceStatus::Ready(_));

        let mut list = v_flex().gap_3().p_3();
        if let Some(panel) = self.render_setup(&theme, cx) {
            list = list.child(panel);
        }
        if let Some(panel) = self.render_guide(&theme, cx) {
            list = list.child(panel);
        }
        if let Some(panel) = self.render_appearance_panel(&theme, cx) {
            list = list.child(panel);
        }
        if let Some(panel) = self.render_key_panel(&theme, cx) {
            list = list.child(panel);
        }
        if self.settings.continuous {
            let jev = self.settings.jev_prefilter;
            list = list.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().flex_1().min_w_0().child(if jev {
                        format!(
                            "When you pause after a sentence, Jev grades the paragraph; checks run above {:.0}%.",
                            self.settings.jev_threshold * 100.
                        )
                    } else {
                        "Every changed paragraph is checked.".to_string()
                    }))
                    .child(div().flex_none().child(
                        Button::new("jev-toggle")
                            .label(if jev {
                                "Jev pre-filter: on"
                            } else {
                                "Jev pre-filter: off"
                            })
                            .xsmall()
                            .when(jev, |b| b.primary())
                            .when(!jev, |b| b.outline())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_jev_prefilter(!jev, window, cx)
                            })),
                    )),
            );
        }
        if self.settings.continuous
            && self.settings.jev_prefilter
            && let Some(panel) = self.render_jev_report(&theme)
        {
            list = list.child(panel);
        }
        if let Some(panel) = self.render_review_panel(&theme, cx) {
            list = list.child(panel);
        }
        let cards = self.runs.iter().filter(|r| !r.background).count();
        if cards == 0 && !self.findings.iter().any(Finding::visible) {
            let message: SharedString = match &self.service_status {
                ServiceStatus::Starting => "Starting the assistant…".into(),
                ServiceStatus::Ready(_) if self.settings.continuous => format!(
                    "Checking paragraphs as you write. Suggestions show up here when you pause.\n\nOr select a passage and choose an action above: {} copy-edit · {} fact-check · {} references",
                    shortcut("E"),
                    shortcut("⇧C"),
                    shortcut("⇧R"),
                )
                .into(),
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
        // Fact-checks of the text under the cursor, moved up from the list
        // of the document's fact-checks below. Ones in an open run's card
        // are left there, so no verdict shows twice.
        let at_cursor: Vec<u64> = self
            .claims_at_cursor(cx)
            .into_iter()
            .filter(|id| self.finding(*id).is_some_and(|f| f.run.is_none()))
            .collect();
        if !at_cursor.is_empty() {
            list = list.child(section_heading("At the cursor", &theme));
            for id in &at_cursor {
                if let Some(f) = self.finding(*id) {
                    list = list.child(self.render_finding("cursor", f, &theme, cx));
                }
            }
        }
        for run in self.runs.iter().filter(|r| !r.background) {
            list = list.child(self.render_run(run, &theme, cx));
        }
        // Edits and references found while writing, in document order.
        let mut written: Vec<&Finding> = self
            .findings
            .iter()
            .filter(|f| !f.is_claim() && f.run.is_none() && f.visible())
            .collect();
        written.sort_by_key(|f| f.anchor);
        let written_edits: Vec<u64> = written
            .iter()
            .filter(|f| matches!(f.kind, FindingKind::Edit(_)))
            .map(|f| f.id)
            .collect();
        if !written.is_empty() {
            list = list.child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(section_heading(
                        &format!("Suggestions ({})", written.len()),
                        &theme,
                    ))
                    .when(written_edits.len() > 1, |d| {
                        let ids = written_edits.clone();
                        d.child(
                            Button::new("accept-written")
                                .label("Accept all")
                                .xsmall()
                                .primary()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.accept_edits(&ids, window, cx)
                                })),
                        )
                    }),
            );
            for f in written {
                list = list.child(self.render_finding("written", f, &theme, cx));
            }
        }
        // Fact-checks kept with the text after their run's card was closed,
        // or saved from an earlier session.
        let mut kept: Vec<&Finding> = self
            .findings
            .iter()
            .filter(|f| f.is_claim() && f.run.is_none() && f.visible())
            .filter(|f| !at_cursor.contains(&f.id))
            .collect();
        kept.sort_by_key(|f| f.anchor);
        if !kept.is_empty() {
            let heading = if at_cursor.is_empty() {
                "Fact-checks in this document"
            } else {
                "Other fact-checks in this document"
            };
            list = list.child(section_heading(
                &format!("{heading} ({})", kept.len()),
                &theme,
            ));
            for f in kept {
                list = list.child(self.render_finding("kept", f, &theme, cx));
            }
        }

        if let Some(panel) = self.render_review_suggestion(&theme, cx) {
            list = list.child(panel);
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
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("continuous")
                            .label(if self.continuous.is_checking() {
                                "As you write ●"
                            } else {
                                "As you write"
                            })
                            .small()
                            .when(self.settings.continuous, |b| b.primary())
                            .when(!self.settings.continuous, |b| b.ghost())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_continuous(!this.settings.continuous, cx)
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
        let findings: Vec<&Finding> = run
            .items
            .iter()
            .filter_map(|id| self.finding(*id))
            .collect();
        let open_edits: Vec<u64> = findings
            .iter()
            .filter(|f| f.visible() && matches!(f.kind, FindingKind::Edit(_)))
            .map(|f| f.id)
            .collect();
        // Findings whose text is gone for now; undo can bring them back.
        let hidden = findings
            .iter()
            .filter(|f| !f.live && f.state == FindingState::Open)
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
                if open_edits.len() > 1 {
                    let ids = open_edits.clone();
                    body = body.child(
                        Button::new(("accept-all", id))
                            .label(format!("Accept all {}", open_edits.len()))
                            .xsmall()
                            .primary()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.accept_edits(&ids, window, cx)
                            })),
                    );
                }
                for f in findings
                    .iter()
                    .filter(|f| f.live || f.state != FindingState::Open)
                {
                    body = body.child(self.render_finding("run", f, theme, cx));
                }
                if hidden > 0 {
                    body = body.child(div().text_xs().text_color(theme.muted_foreground).child(
                        if hidden == 1 {
                            "1 more refers to text that has changed".to_string()
                        } else {
                            format!("{hidden} more refer to text that has changed")
                        },
                    ));
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

    fn render_finding(
        &self,
        section: &str,
        f: &Finding,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fid = f.id;
        let key = |name: &str| ElementId::NamedInteger(format!("{name}-{section}").into(), fid);
        let open = f.visible();
        let state_note = match f.state {
            FindingState::Open if !f.live => Some("Its text has changed"),
            FindingState::Open => None,
            FindingState::Accepted if matches!(f.kind, FindingKind::Reference(_)) => {
                Some("Inserted")
            }
            FindingState::Accepted => Some("Accepted"),
            FindingState::Dismissed => Some("Dismissed"),
        };

        let content = match &f.kind {
            FindingKind::Edit(e) => v_flex()
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
            FindingKind::Claim(c) => {
                let mut sources = h_flex().flex_wrap().gap_2();
                for (si, source) in c.sources.iter().enumerate() {
                    let url = source.url.clone();
                    sources = sources.child(
                        div()
                            .id(ElementId::NamedInteger(
                                format!("src-{section}-{fid}").into(),
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
            FindingKind::Reference(r) => {
                let url = r.url.clone();
                let meta = r.meta();
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
                                    .id(key("ref"))
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

        let is_edit = matches!(f.kind, FindingKind::Edit(_));
        let is_reference = matches!(f.kind, FindingKind::Reference(_));
        v_flex()
            .id(key("finding"))
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(theme.muted.opacity(0.4))
            .when(!open, |d| d.opacity(0.55))
            .when(open, |d| {
                d.cursor_pointer().hover(|s| s.bg(theme.muted)).on_click(
                    cx.listener(move |this, _, window, cx| this.reveal_finding(fid, window, cx)),
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
                                Button::new(key("accept"))
                                    .label("Accept")
                                    .xsmall()
                                    .primary()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.accept_edits(&[fid], window, cx)
                                    })),
                            )
                        })
                        .when(is_reference, |row| {
                            row.child(
                                Button::new(key("footnote"))
                                    .label("Insert footnote")
                                    .xsmall()
                                    .primary()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.insert_reference(fid, true, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(key("link"))
                                    .label("Insert link")
                                    .xsmall()
                                    .outline()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.insert_reference(fid, false, window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new(key("dismiss"))
                                .label("Dismiss")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.dismiss_finding(fid, cx)
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Workspace {
    /// Jev's latest grades: each score, and whether it earned a check.
    fn render_jev_report(&self, theme: &Theme) -> Option<AnyElement> {
        let grading = self.continuous.is_grading();
        let report = self.continuous.last_grade();
        if !grading && report.is_none() {
            return None;
        }
        let threshold = self.settings.jev_threshold;
        let mut panel = v_flex()
            .gap_1()
            .p_2()
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .text_xs()
            .child(section_heading("Jev", theme));
        if grading {
            panel = panel.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("Grading the paragraph you just wrote…"),
            );
        }
        if let Some(report) = report {
            panel = panel.child(
                div()
                    .italic()
                    .text_color(theme.muted_foreground)
                    .child(format!("“{}”", report.excerpt)),
            );
            let rows = verdicts(&report.grades, &self.continuous_kinds(), threshold);
            for (kind, score, verdict) in &rows {
                let (note, color): (String, _) = match (verdict, report.status(*kind)) {
                    (Verdict::Check, None) => ("queued".into(), theme.foreground),
                    (Verdict::Check, Some(CheckStatus::Running)) => {
                        ("checking…".into(), theme.foreground)
                    }
                    (Verdict::Check, Some(CheckStatus::Done(0))) => {
                        ("checked: nothing found".into(), theme.foreground)
                    }
                    (Verdict::Check, Some(CheckStatus::Done(n))) => {
                        (format!("checked: {n} below"), theme.foreground)
                    }
                    (Verdict::Check, Some(CheckStatus::Failed(m))) => {
                        (format!("didn't run: {m}"), theme.danger)
                    }
                    (Verdict::Skip, _) => ("not needed".into(), theme.muted_foreground),
                    (Verdict::Off, _) => ("off as you write".into(), theme.muted_foreground),
                };
                panel = panel.child(
                    h_flex()
                        .gap_2()
                        .text_color(color)
                        .when(*verdict == Verdict::Check, |d| {
                            d.font_weight(FontWeight::SEMIBOLD)
                        })
                        .child(div().w(px(80.)).child(task_label(*kind)))
                        .child(div().w(px(40.)).child(format!("{:.0}%", score * 100.)))
                        .child(div().flex_1().min_w_0().child(note)),
                );
            }
            if !rows.iter().any(|(_, _, v)| *v == Verdict::Check) {
                panel = panel.child(div().pt_1().text_color(theme.muted_foreground).child(
                    format!(
                        "Nothing scored above {:.0}%, so no checks are needed.",
                        threshold * 100.
                    ),
                ));
            }
        }
        Some(panel.into_any_element())
    }
}

pub(crate) fn task_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::CopyEdit => "Copy-edit",
        TaskKind::FactCheck => "Fact-check",
        TaskKind::FindReferences => "References",
    }
}

fn section_heading(title: &str, theme: &Theme) -> Div {
    div()
        .pt_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .child(title.to_string())
}

/// A shortcut hint for the empty sidebar, such as "⌘E".
fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{}", key.replace('⇧', "Shift+"))
    }
}
