//! A review of the whole document: copy-edit, fact-check and references
//! over every paragraph, rather than one paragraph at a time as you write.
//!
//! It never starts by itself. With the Jev pre-filter on, Jev grades now and
//! then, after a long pause, whether the draft looks ready for a full
//! look-over; above the threshold the sidebar suggests a review, and the
//! writer starts it with a button (or Assistant ▸ Review Whole Document).
//!
//! Each kind of help runs in its own lane, one request at a time. A document
//! too long for one request is split into chunks on paragraph boundaries,
//! each sent with the whole document as context when that fits, or else with
//! the post's headings and the text just before and after it. Chunks are
//! picked from the paragraphs not yet reviewed in this pass, so a paragraph
//! edited while the pass runs is reviewed again in its new form.

use std::collections::HashSet;
use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use wingman_client::TaskKind;

use crate::ReviewDocument;
use crate::continuous::{fingerprint, paragraphs};
use crate::jev::JevError;
use crate::runs::MAX_CONTEXT_BYTES;
use crate::suggestions::relocate;
use crate::workspace::Workspace;

/// How long writing must pause before Jev grades the whole document.
pub const READINESS_PAUSE: Duration = Duration::from_secs(30);

/// Too little to be worth a full review, or grading.
const MIN_PARAGRAPHS: usize = 3;
const MIN_WORDS: usize = 150;

/// Grade again only after this much has changed since the last grade.
const REGRADE_PARAGRAPHS: usize = 2;
const REGRADE_WORDS: usize = 50;

/// The most text sent as one chunk: copy-editing reads every word, while
/// fact-checks and references search the web for each claim, so they get
/// smaller chunks.
fn budget(kind: TaskKind) -> usize {
    match kind {
        TaskKind::CopyEdit => 16_000,
        TaskKind::FactCheck | TaskKind::FindReferences => 6_000,
    }
}

/// How much text either side of a chunk goes with it, when the whole
/// document is too long to send as context.
const NEIGHBOURS: usize = 3_000;

const KINDS: [TaskKind; 3] = [
    TaskKind::CopyEdit,
    TaskKind::FactCheck,
    TaskKind::FindReferences,
];

#[derive(Default)]
pub struct Review {
    pass: Option<Pass>,
    /// The last pass's outcome, until the writer closes it.
    last: Option<Outcome>,
    /// Jev's last grade of the document's readiness for a full review.
    readiness: Option<f32>,
    /// What the document looked like at that grade: its paragraphs'
    /// fingerprints and its word count.
    graded: HashSet<u64>,
    graded_words: usize,
    /// Whether the sidebar suggests a review now.
    suggest: bool,
    grading: bool,
    _grading_task: Option<Task<()>>,
    timer: Option<Task<()>>,
}

struct Pass {
    lanes: Vec<Lane>,
    /// (kind, paragraph fingerprint) reviewed in this pass.
    reviewed: HashSet<(TaskKind, u64)>,
    started_findings: u64,
    failed: usize,
    failure: Option<String>,
}

struct Lane {
    kind: TaskKind,
    /// The run in flight: its id, the text it was sent, and the paragraphs
    /// it covers.
    running: Option<(usize, String, Vec<String>)>,
    parts_done: usize,
    /// Parts done plus the parts left, at the last count.
    parts: usize,
    finished: bool,
}

/// How a pass went, for the sidebar.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub parts: usize,
    pub findings: usize,
    pub failed: usize,
    pub failure: Option<String>,
}

impl Review {
    pub fn is_running(&self) -> bool {
        self.pass.is_some()
    }

    pub fn is_suggested(&self) -> bool {
        self.suggest && self.pass.is_none()
    }

    pub fn readiness(&self) -> Option<f32> {
        self.readiness
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        self.last.as_ref()
    }

    /// Each lane's kind, parts done and parts in all, and whether it's done.
    pub fn progress(&self) -> Vec<(TaskKind, usize, usize, bool)> {
        self.pass
            .iter()
            .flat_map(|p| &p.lanes)
            .map(|l| (l.kind, l.parts_done, l.parts, l.finished))
            .collect()
    }

    /// A new document: forget what was graded and reviewed.
    pub fn reset(&mut self) {
        let pass = self.pass.take();
        *self = Review::default();
        // Runs still in flight are cancelled by the caller.
        drop(pass);
    }

    fn running_ids(&self) -> Vec<usize> {
        self.pass
            .iter()
            .flat_map(|p| &p.lanes)
            .filter_map(|l| l.running.as_ref().map(|(id, _, _)| *id))
            .collect()
    }
}

/// The next chunk for `kind`: consecutive paragraphs not yet reviewed, from
/// the first such one, up to the kind's budget (a longer paragraph is a chunk
/// on its own). Returns the chunk's span and its paragraphs, and how many
/// chunks are left in all.
fn next_chunk(
    text: &str,
    kind: TaskKind,
    reviewed: &HashSet<(TaskKind, u64)>,
) -> Option<(Range<usize>, Vec<Range<usize>>, usize)> {
    let all = paragraphs(text);
    let pending = |p: &Range<usize>| !reviewed.contains(&(kind, fingerprint(&text[p.clone()])));
    // Runs of consecutive pending paragraphs, split to fit the budget.
    let mut chunks: Vec<Vec<Range<usize>>> = Vec::new();
    let mut current: Vec<Range<usize>> = Vec::new();
    for p in all {
        if !pending(&p) {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(first) = current.first()
            && p.end - first.start > budget(kind)
        {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(p);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    let count = chunks.len();
    let first = chunks.into_iter().next()?;
    let span = first[0].start..first[first.len() - 1].end;
    Some((span, first, count))
}

/// What goes with a chunk as context: nothing when the chunk is nearly the
/// whole document, the whole document when it's small enough, or else the
/// post's headings and the text either side of the chunk.
pub fn chunk_context(text: &str, chunk: &Range<usize>) -> Option<String> {
    if chunk.len() * 10 >= text.len() * 9 {
        return None;
    }
    if text.len() <= MAX_CONTEXT_BYTES {
        return Some(text.to_string());
    }
    let headings: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with('#'))
        .collect();
    let floor = |mut i: usize| {
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    // Start and end the neighbouring text on paragraph breaks where possible.
    let before_start = floor(chunk.start.saturating_sub(NEIGHBOURS));
    let before_start = text[before_start..chunk.start]
        .find("\n\n")
        .map_or(before_start, |i| before_start + i + 2);
    let after_end = floor((chunk.end + NEIGHBOURS).min(text.len()));
    let after_end = text[chunk.end..after_end]
        .rfind("\n\n")
        .map_or(after_end, |i| chunk.end + i);
    let mut out =
        String::from("The passage is one part of a longer post, reviewed a part at a time.\n\n");
    if !headings.is_empty() {
        out.push_str("The post's headings:\n");
        for h in headings {
            out.push_str(h.trim());
            out.push('\n');
        }
        out.push('\n');
    }
    let before = text[before_start..chunk.start].trim();
    if !before.is_empty() {
        out.push_str(&format!("Just before the passage:\n{before}\n\n"));
    }
    let after = text[chunk.end..after_end].trim();
    if !after.is_empty() {
        out.push_str(&format!("Just after the passage:\n{after}\n"));
    }
    Some(out)
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

impl Workspace {
    pub(crate) fn on_review_document(
        &mut self,
        _: &ReviewDocument,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_review(cx);
    }

    /// Starts a review of the whole document.
    pub(crate) fn start_review(&mut self, cx: &mut Context<Self>) {
        if self.assistant.is_none() {
            self.set_notice("The assistant isn't running", cx);
            return;
        }
        if self.review.is_running() {
            return;
        }
        let text = self.text(cx);
        if paragraphs(&text).is_empty() {
            self.set_notice("There's nothing to review yet", cx);
            return;
        }
        log::info!(
            target: "review",
            "reviewing the whole document: {} paragraphs, {}",
            paragraphs(&text).len(),
            wingman_log::text(&text)
        );
        self.review.suggest = false;
        self.review.last = None;
        self.review.pass = Some(Pass {
            lanes: KINDS
                .iter()
                .map(|&kind| Lane {
                    kind,
                    running: None,
                    parts_done: 0,
                    parts: 0,
                    finished: false,
                })
                .collect(),
            reviewed: HashSet::new(),
            started_findings: self.next_finding_id,
            failed: 0,
            failure: None,
        });
        for lane in 0..KINDS.len() {
            self.review_step(lane, cx);
        }
        cx.notify();
    }

    /// Another document: stop any review and forget the last grade.
    pub(crate) fn reset_review(&mut self, cx: &mut Context<Self>) {
        let ids = self.review.running_ids();
        self.review.reset();
        for id in ids {
            self.cancel_run(id, cx);
        }
    }

    pub(crate) fn cancel_review(&mut self, cx: &mut Context<Self>) {
        let ids = self.review.running_ids();
        self.review.pass = None;
        for id in ids {
            self.cancel_run(id, cx);
        }
        self.set_notice("Stopped the review", cx);
    }

    /// Starts a lane's next chunk, or finishes the lane.
    fn review_step(&mut self, lane: usize, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let Some(pass) = self.review.pass.as_mut() else {
            return;
        };
        let kind = pass.lanes[lane].kind;
        let Some((span, paras, left)) = next_chunk(&text, kind, &pass.reviewed) else {
            pass.lanes[lane].finished = true;
            pass.lanes[lane].parts = pass.lanes[lane].parts_done;
            if pass.lanes.iter().all(|l| l.finished) {
                self.finish_review(cx);
            }
            return;
        };
        pass.lanes[lane].parts = pass.lanes[lane].parts_done + left;
        log::debug!(
            target: "review",
            "{kind:?}: part {} of {} ({} paragraphs)",
            pass.lanes[lane].parts_done + 1,
            pass.lanes[lane].parts,
            paras.len()
        );
        let sent = text[span.clone()].to_string();
        let covered: Vec<String> = paras.iter().map(|p| text[p.clone()].to_string()).collect();
        let context = chunk_context(&text, &span);
        match self.start_run_with(kind, span, true, context, cx) {
            Some(id) => {
                self.mark_review_run(id);
                if let Some(pass) = self.review.pass.as_mut() {
                    pass.lanes[lane].running = Some((id, sent, covered));
                }
            }
            None => {
                log::warn!(target: "review", "{kind:?}: couldn't start a part; giving up on it");
                // Couldn't start (the reason is on the status bar): give up
                // on this lane rather than loop.
                if let Some(pass) = self.review.pass.as_mut() {
                    pass.lanes[lane].finished = true;
                    pass.failed += 1;
                }
                if self
                    .review
                    .pass
                    .as_ref()
                    .is_some_and(|p| p.lanes.iter().all(|l| l.finished))
                {
                    self.finish_review(cx);
                }
            }
        }
    }

    /// Called when one of the review's runs ends, however it ended.
    pub(crate) fn review_run_ended(
        &mut self,
        id: usize,
        failure: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(pass) = self.review.pass.as_mut() else {
            return;
        };
        let Some(lane) = pass
            .lanes
            .iter()
            .position(|l| l.running.as_ref().is_some_and(|(r, _, _)| *r == id))
        else {
            return;
        };
        let (_, _, covered) = pass.lanes[lane].running.take().unwrap();
        let kind = pass.lanes[lane].kind;
        match failure.as_deref() {
            // Its text changed under it: review it again as it is now.
            Some("cancelled") => {}
            other => {
                if let Some(message) = other {
                    pass.failed += 1;
                    pass.failure = Some(message.to_string());
                }
                pass.lanes[lane].parts_done += 1;
                for p in &covered {
                    pass.reviewed.insert((kind, fingerprint(p)));
                }
                if other.is_none() {
                    // No need to check these again as you write.
                    for p in &covered {
                        self.continuous.mark(kind, p);
                    }
                }
            }
        }
        self.review_step(lane, cx);
        cx.notify();
    }

    fn finish_review(&mut self, cx: &mut Context<Self>) {
        let Some(pass) = self.review.pass.take() else {
            return;
        };
        let findings = self
            .findings
            .iter()
            .filter(|f| f.id >= pass.started_findings && f.visible())
            .count();
        let parts: usize = pass.lanes.iter().map(|l| l.parts_done).sum();
        log::info!(
            target: "review",
            "review finished: {parts} parts, {findings} findings, {} failed{}",
            pass.failed,
            pass.failure
                .as_deref()
                .map(|f| format!(" (last: {f})"))
                .unwrap_or_default()
        );
        self.review.last = Some(Outcome {
            parts,
            findings,
            failed: pass.failed,
            failure: pass.failure,
        });
        self.set_notice("Finished reviewing the whole document", cx);
    }

    /// After every edit: keep the review's runs on their text, cancelling
    /// any whose text has changed, and wait for a long pause to grade the
    /// document's readiness again.
    pub(crate) fn review_edited(&mut self, cx: &mut Context<Self>) {
        if self.review.is_running() {
            let text = self.text(cx);
            let running: Vec<(usize, String)> = self
                .review
                .pass
                .iter()
                .flat_map(|p| &p.lanes)
                .filter_map(|l| l.running.as_ref().map(|(id, sent, _)| (*id, sent.clone())))
                .collect();
            for (id, sent) in running {
                let start = self.run_passage(id).map(|p| p.start).unwrap_or(0);
                match relocate(&text, &sent, start, 4_000) {
                    Some(range) => self.set_run_passage(id, range),
                    None => self.cancel_run(id, cx),
                }
            }
        }
        self.schedule_readiness(cx);
    }

    fn readiness_enabled(&self) -> bool {
        self.settings.continuous && self.settings.jev_prefilter
    }

    pub(crate) fn schedule_readiness(&mut self, cx: &mut Context<Self>) {
        if !self.readiness_enabled() {
            self.review.timer = None;
            return;
        }
        self.review.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(READINESS_PAUSE).await;
            let _ = this.update(cx, |this, cx| this.readiness_tick(cx));
        }));
    }

    /// Asks Jev whether the document looks ready for a full review, if
    /// writing has paused long enough and enough has changed since it last
    /// asked.
    fn readiness_tick(&mut self, cx: &mut Context<Self>) {
        let now: Instant = cx.background_executor().now();
        if !self.readiness_enabled()
            || self.review.grading
            || self.review.is_running()
            || self.continuous.jev_blocked()
            || self
                .continuous
                .last_edit()
                .is_some_and(|t| now.saturating_duration_since(t) < READINESS_PAUSE)
        {
            return;
        }
        // Never read the Keychain from here: grading waits for the key.
        let Some(Some(key)) = self.continuous.cached_key() else {
            return;
        };
        let text = self.text(cx);
        let paras = paragraphs(&text);
        let count = words(&text);
        if paras.len() < MIN_PARAGRAPHS || count < MIN_WORDS {
            return;
        }
        let fps: HashSet<u64> = paras
            .iter()
            .map(|p| fingerprint(&text[p.clone()]))
            .collect();
        if self.review.readiness.is_some() {
            let changed = fps.difference(&self.review.graded).count();
            if changed < REGRADE_PARAGRAPHS
                && count.abs_diff(self.review.graded_words) < REGRADE_WORDS
            {
                return;
            }
        }
        let model = self.settings.jev_model.clone();
        log::debug!(
            target: "jev",
            "asking about readiness: {} paragraphs, {count} words",
            paras.len()
        );
        let grader = self.grader.clone();
        self.review.grading = true;
        let grading = cx
            .background_executor()
            .spawn(async move { grader.grade_readiness(&key, &model, &text) });
        self.review._grading_task = Some(cx.spawn(async move |this, cx| {
            let result = grading.await;
            let _ = this.update(cx, |this, cx| this.readiness_done(fps, count, result, cx));
        }));
    }

    fn readiness_done(
        &mut self,
        fps: HashSet<u64>,
        count: usize,
        result: Result<f32, JevError>,
        cx: &mut Context<Self>,
    ) {
        self.review.grading = false;
        self.review._grading_task = None;
        match result {
            Ok(score) => {
                log::info!(
                    target: "jev",
                    "readiness for a full review: {:.0}% ({} at {:.0}%)",
                    score * 100.,
                    if score > self.settings.jev_threshold { "suggesting a review" } else { "not yet" },
                    self.settings.jev_threshold * 100.
                );
                self.review.readiness = Some(score);
                self.review.graded = fps;
                self.review.graded_words = count;
                self.review.suggest = score > self.settings.jev_threshold;
            }
            Err(JevError::BadKey) => {
                log::error!(target: "jev", "TypeSafe rejected the key while grading readiness");
                self.continuous.block_until_key();
                self.set_notice(
                    "TypeSafe didn't accept the API key. Set it again: Assistant ▸ Set TypeSafe API Key…",
                    cx,
                );
            }
            // Busy or unreachable: ask again after the next long pause.
            Err(e) => {
                log::warn!(target: "jev", "readiness grading failed ({e}); asking again later")
            }
        }
        cx.notify();
    }

    /// The suggestion to review the whole document, when Jev thinks the
    /// draft is ready. It goes below everything else in the sidebar, so it
    /// appears without moving the cards above it.
    pub(crate) fn render_review_suggestion(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.review.is_suggested() || self.review.is_running() {
            return None;
        }
        let panel = || {
            v_flex()
                .gap_2()
                .p_2()
                .border_1()
                .border_color(theme.border)
                .rounded_md()
                .text_xs()
        };
        let score = self.review.readiness().unwrap_or_default();
        Some(
            panel()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                    "This draft looks ready for a full look-over (Jev: {:.0}%)",
                    score * 100.
                )))
                .child(div().text_color(theme.muted_foreground).child(
                    "Copy-edit, fact-check and find references across the whole document, in the background.",
                ))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("review-start")
                                .label("Review whole document")
                                .xsmall()
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| this.start_review(cx))),
                        )
                        .child(
                            Button::new("review-later")
                                .label("Not now")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.review.suggest = false;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The review's panel at the top of the sidebar: the pass's progress,
    /// or how it went.
    pub(crate) fn render_review_panel(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = || {
            v_flex()
                .gap_2()
                .p_2()
                .border_1()
                .border_color(theme.border)
                .rounded_md()
                .text_xs()
        };
        if self.review.is_running() {
            let mut lines = v_flex().gap_1();
            for (kind, done, parts, finished) in self.review.progress() {
                let state = if finished {
                    "done".to_string()
                } else if parts > 1 {
                    format!("part {} of {parts}", (done + 1).min(parts))
                } else {
                    "reviewing…".to_string()
                };
                lines = lines.child(
                    h_flex()
                        .gap_2()
                        .text_color(if finished {
                            theme.muted_foreground
                        } else {
                            theme.foreground
                        })
                        .child(div().w(px(80.)).child(crate::runs::task_label(kind)))
                        .child(div().child(state)),
                );
            }
            return Some(
                panel()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Reviewing the whole document"),
                            )
                            .child(
                                Button::new("review-cancel")
                                    .label("Stop")
                                    .xsmall()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_review(cx))),
                            ),
                    )
                    .child(lines)
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child("Keep writing: what it finds shows up below and by the text."),
                    )
                    .into_any_element(),
            );
        }
        // The suggestion to review goes at the bottom of the sidebar, in
        // `render_review_suggestion`, so it can't push down what's being read.
        if self.review.is_suggested() {
            return None;
        }
        let outcome = self.review.outcome()?;
        let mut summary = match outcome.findings {
            0 => "Reviewed the whole document: nothing to suggest.".to_string(),
            1 => "Reviewed the whole document: 1 finding, below and by the text.".to_string(),
            n => format!("Reviewed the whole document: {n} findings, below and by the text."),
        };
        if outcome.failed > 0 {
            summary.push_str(&format!(
                " {} part{} couldn't be reviewed{}",
                outcome.failed,
                if outcome.failed == 1 { "" } else { "s" },
                outcome
                    .failure
                    .as_ref()
                    .map(|m| format!(": {m}"))
                    .unwrap_or_else(|| ".".into())
            ));
        }
        Some(
            panel()
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(div().flex_1().min_w_0().child(summary))
                        .child(
                            Button::new("review-close")
                                .label("×")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.review.last = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{chunk_context, next_chunk};
    use crate::continuous::fingerprint;
    use std::collections::HashSet;
    use wingman_client::TaskKind;

    fn para(n: usize, len: usize) -> String {
        let word = format!("p{n}word ");
        word.repeat(len / word.len() + 1).trim().to_string() + "."
    }

    #[test]
    fn a_short_document_is_one_chunk() {
        let doc = format!(
            "# Title\n\n{}\n\n## Part\n\n{}\n",
            para(1, 200),
            para(2, 200)
        );
        let (span, paras, left) = next_chunk(&doc, TaskKind::CopyEdit, &HashSet::new()).unwrap();
        assert_eq!(left, 1);
        assert_eq!(paras.len(), 2);
        // The chunk runs from the first paragraph to the last, headings and all.
        assert!(doc[span.clone()].contains("## Part"));
        assert_eq!(
            chunk_context(&doc, &span),
            None,
            "the chunk is the document"
        );
    }

    #[test]
    fn a_long_document_is_split_on_paragraphs() {
        let paras: Vec<String> = (0..10).map(|n| para(n, 2_000)).collect();
        let doc = paras.join("\n\n");
        let (span, first, left) = next_chunk(&doc, TaskKind::FactCheck, &HashSet::new()).unwrap();
        // 6,000 bytes a chunk: two 2,000-byte paragraphs and their breaks
        // fit, three don't.
        assert_eq!(first.len(), 2, "{span:?}");
        assert_eq!(left, 5);
        // Reviewed paragraphs are skipped, and break up the runs either side.
        let mut reviewed = HashSet::new();
        for p in [&paras[0], &paras[1], &paras[4]] {
            reviewed.insert((TaskKind::FactCheck, fingerprint(p)));
        }
        let (span, first, left) = next_chunk(&doc, TaskKind::FactCheck, &reviewed).unwrap();
        assert_eq!(&doc[span], format!("{}\n\n{}", paras[2], paras[3]));
        assert_eq!(first.len(), 2);
        assert_eq!(left, 4, "2-3, 5-6, 7-8, 9");
        // Other kinds still see every paragraph.
        assert!(next_chunk(&doc, TaskKind::CopyEdit, &reviewed).unwrap().1[0].start == 0);
    }

    #[test]
    fn a_long_documents_chunks_carry_headings_and_neighbours() {
        let mut doc = String::from("# The post\n\n");
        for n in 0..30 {
            if n % 10 == 0 {
                doc.push_str(&format!("## Section {}\n\n", n / 10));
            }
            doc.push_str(&para(n, 2_000));
            doc.push_str("\n\n");
        }
        let start = doc.find("p15word").unwrap();
        let end = start + para(15, 2_000).len();
        let context = chunk_context(&doc, &(start..end)).unwrap();
        for part in ["# The post", "## Section 0", "## Section 1", "## Section 2"] {
            assert!(context.contains(part), "{part}");
        }
        assert!(context.contains("Just before the passage:\np14word"));
        assert!(context.contains("Just after the passage:\np16word"));
        assert!(!context.contains("p15word"), "not the passage itself");
        assert!(!context.contains("p10word"), "not the far text");
        assert!(context.len() < 8_000);
    }
}
