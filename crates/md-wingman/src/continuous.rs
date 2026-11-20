//! Continuous mode: when typing (or a dictation burst) pauses, check the
//! paragraphs that changed, one at a time, and show what's found as it comes.
//!
//! Each paragraph's text is fingerprinted once it has been checked, so only
//! new or changed paragraphs cost a request. When the mode is turned on, or a
//! document is opened, the paragraphs already there count as checked: it
//! follows what you write, and the sidebar buttons check the rest on demand.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::*;
use wingman_client::TaskKind;

use crate::suggestions::relocate;
use crate::workspace::Workspace;
use crate::ToggleContinuous;

/// Paragraphs with fewer words than this aren't worth a request.
const MIN_WORDS: usize = 6;

#[derive(Default)]
pub struct Continuous {
    /// (task, fingerprint) pairs already checked.
    checked: HashSet<(u8, u64)>,
    /// The pending check, restarted by every edit.
    timer: Option<Task<()>>,
    last_edit: Option<Instant>,
    /// The background run in flight, and the text it was sent.
    running: Option<(usize, String)>,
}

impl Continuous {
    pub fn running_id(&self) -> Option<usize> {
        self.running.as_ref().map(|(id, _)| *id)
    }

    pub fn is_checking(&self) -> bool {
        self.running.is_some()
    }

    fn mark(&mut self, kind: TaskKind, text: &str) {
        self.checked.insert((kind_key(kind), fingerprint(text)));
    }

    fn is_checked(&self, kind: TaskKind, text: &str) -> bool {
        self.checked.contains(&(kind_key(kind), fingerprint(text)))
    }

    /// Counts every paragraph in `text` as checked.
    fn baseline(&mut self, text: &str, kinds: &[TaskKind]) {
        for range in paragraphs(text) {
            for &kind in kinds {
                self.mark(kind, &text[range.clone()]);
            }
        }
    }
}

fn kind_key(kind: TaskKind) -> u8 {
    match kind {
        TaskKind::CopyEdit => 0,
        TaskKind::FactCheck => 1,
        TaskKind::FindReferences => 2,
    }
}

/// A fingerprint of a paragraph that ignores how it's wrapped, so reflowing
/// doesn't count as a change.
pub fn fingerprint(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    for word in text.split_whitespace() {
        word.hash(&mut hasher);
    }
    hasher.finish()
}

/// The prose paragraphs of a Markdown document: blocks between blank lines,
/// leaving out front matter, code, headings, tables, HTML, rules, link
/// definitions and anything too short to check.
pub fn paragraphs(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut lines = Vec::new();
    for line in text.split_inclusive('\n') {
        lines.push((offset, line));
        offset += line.len();
    }

    let mut i = 0;
    // Front matter: a `---` or `+++` block at the very top.
    if let Some((_, first)) = lines.first() {
        let fence = first.trim_end();
        if fence == "---" || fence == "+++" {
            if let Some(end) = lines[1..].iter().position(|(_, l)| l.trim_end() == fence) {
                i = end + 2;
            }
        }
    }

    let mut block: Option<Range<usize>> = None;
    let mut fence: Option<&str> = None;
    let flush = |block: &mut Option<Range<usize>>, out: &mut Vec<Range<usize>>| {
        if let Some(range) = block.take() {
            let slice = &text[range.clone()];
            let trimmed = slice.trim_end();
            if is_prose(trimmed) {
                out.push(range.start..range.start + trimmed.len());
            }
        }
    };
    while i < lines.len() {
        let (start, line) = lines[i];
        let trimmed = line.trim();
        i += 1;
        if let Some(f) = fence {
            if trimmed.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            flush(&mut block, &mut out);
            fence = Some(&trimmed[..3]);
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut block, &mut out);
            continue;
        }
        // A heading ends a paragraph and isn't one.
        if trimmed.starts_with('#') {
            flush(&mut block, &mut out);
            continue;
        }
        match &mut block {
            Some(range) => range.end = start + line.len(),
            None => block = Some(start..start + line.len()),
        }
    }
    flush(&mut block, &mut out);
    out
}

fn is_prose(block: &str) -> bool {
    let first = block.lines().next().unwrap_or("");
    let lead = first.trim_start();
    if first.starts_with("    ") || first.starts_with('\t') {
        return false; // indented code
    }
    if lead.starts_with('|') || lead.starts_with('<') {
        return false; // a table or HTML
    }
    if lead.starts_with('[') && lead.contains("]:") {
        return false; // link or footnote definitions
    }
    let rule: String = lead.chars().filter(|c| !c.is_whitespace()).collect();
    if rule.len() >= 3
        && (rule.chars().all(|c| c == '-')
            || rule.chars().all(|c| c == '*')
            || rule.chars().all(|c| c == '_'))
    {
        return false;
    }
    block.split_whitespace().count() >= MIN_WORDS
}

/// Whether a paragraph looks finished: it ends a sentence.
pub fn finished(paragraph: &str) -> bool {
    let end = paragraph
        .trim_end()
        .trim_end_matches(['"', '\'', ')', ']', '*', '_', '”', '’']);
    end.ends_with(['.', '!', '?', ':', '…'])
}

impl Workspace {
    pub(crate) fn continuous_kinds(&self) -> Vec<TaskKind> {
        let mut kinds = vec![TaskKind::CopyEdit];
        if self.settings.continuous_fact_check {
            kinds.push(TaskKind::FactCheck);
        }
        kinds
    }

    pub(crate) fn on_toggle_continuous(
        &mut self,
        _: &ToggleContinuous,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_continuous(!self.settings.continuous, cx);
    }

    pub(crate) fn set_continuous(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.continuous = on;
        self.save_settings(cx);
        if on {
            self.continuous_baseline(cx);
            self.set_notice("Checking paragraphs as you write", cx);
        } else {
            self.continuous.timer = None;
            if let Some(id) = self.continuous.running_id() {
                self.cancel_run(id, cx);
            }
            self.set_notice("Stopped checking as you write", cx);
        }
        cx.notify();
    }

    /// Counts the document as it is now as checked.
    pub(crate) fn continuous_baseline(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let kinds = self.continuous_kinds();
        self.continuous.baseline(&text, &kinds);
    }

    /// After every edit: restart the pause timer, and give up on a check
    /// whose paragraph has changed.
    pub(crate) fn continuous_edited(&mut self, cx: &mut Context<Self>) {
        if !self.settings.continuous {
            return;
        }
        self.continuous.last_edit = Some(Instant::now());
        if let Some((id, sent)) = self.continuous.running.clone() {
            let text = self.text(cx);
            let start = self.run_passage(id).map(|p| p.start).unwrap_or(0);
            match relocate(&text, &sent, start, 2_000) {
                Some(range) => self.set_run_passage(id, range),
                None => self.cancel_run(id, cx),
            }
        }
        self.schedule_continuous(cx);
    }

    fn delay(&self) -> Duration {
        Duration::from_millis(self.settings.continuous_delay_ms)
    }

    fn schedule_continuous(&mut self, cx: &mut Context<Self>) {
        let delay = self.delay();
        self.continuous.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| this.continuous_tick(cx));
        }));
    }

    /// Starts the next check, if typing has paused and nothing is running.
    pub(crate) fn continuous_tick(&mut self, cx: &mut Context<Self>) {
        if !self.settings.continuous
            || self.assistant.is_none()
            || self.continuous.is_checking()
            || self
                .continuous
                .last_edit
                .is_some_and(|t| t.elapsed() < self.delay())
        {
            return;
        }
        let text = self.text(cx);
        let cursor = self.editor().read(cx).selected_range().start;
        let kinds = self.continuous_kinds();
        // Paragraphs in order, the one being written last, and only once it
        // ends a sentence: dictation often pauses mid-paragraph.
        let mut candidates: Vec<Range<usize>> = Vec::new();
        let mut current = None;
        for p in paragraphs(&text) {
            if (p.start..=p.end).contains(&cursor) {
                current = Some(p);
            } else {
                candidates.push(p);
            }
        }
        if let Some(p) = current.filter(|p| finished(&text[p.clone()])) {
            candidates.push(p);
        }
        let next = candidates.into_iter().find_map(|p| {
            kinds
                .iter()
                .find(|&&k| !self.continuous.is_checked(k, &text[p.clone()]))
                .map(|&k| (k, p))
        });
        let Some((kind, passage)) = next else {
            return;
        };
        let sent = text[passage.clone()].to_string();
        self.continuous.mark(kind, &sent);
        if let Some(id) = self.start_run_on(kind, passage, true, cx) {
            self.continuous.running = Some((id, sent));
        }
        cx.notify();
    }

    /// Called when a background run ends, however it ended.
    pub(crate) fn continuous_run_ended(
        &mut self,
        id: usize,
        kind: TaskKind,
        failure: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some((running, sent)) = self.continuous.running.take() else {
            return;
        };
        if running != id {
            self.continuous.running = Some((running, sent));
            return;
        }
        match failure {
            None => self.continuous_tick(cx),
            Some(message) => {
                // Try it again next time, unless the text moved on anyway.
                self.continuous
                    .checked
                    .remove(&(kind_key(kind), fingerprint(&sent)));
                if message != "cancelled" {
                    self.set_notice(format!("Paused checking as you write: {message}"), cx);
                } else {
                    self.continuous_tick(cx);
                }
            }
        }
    }

    /// Accepted edits change their paragraph; that shouldn't count as new
    /// writing to check again. Call with the text before and after.
    pub(crate) fn continuous_carry_over(
        &mut self,
        before: &str,
        span: Range<usize>,
        after: &str,
        new_span: Range<usize>,
    ) {
        let overlapping = |text: &str, span: &Range<usize>| -> Vec<Range<usize>> {
            paragraphs(text)
                .into_iter()
                .filter(|p| p.start <= span.end && span.start <= p.end)
                .collect()
        };
        let old = overlapping(before, &span);
        let new = overlapping(after, &new_span);
        for kind in [TaskKind::CopyEdit, TaskKind::FactCheck] {
            if !old.is_empty()
                && old
                    .iter()
                    .all(|p| self.continuous.is_checked(kind, &before[p.clone()]))
            {
                for p in &new {
                    self.continuous.mark(kind, &after[p.clone()]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, finished, paragraphs};

    fn texts(doc: &str) -> Vec<&str> {
        paragraphs(doc).into_iter().map(|r| &doc[r]).collect()
    }

    #[test]
    fn finds_prose_paragraphs() {
        let doc = "---\ntitle: A post about towers\ndraft: true\n---\n\n# The Eiffel Tower\n\nThe tower was completed in 1889 for the fair.\nIt is about 330 metres tall.\n\n```rust\nlet x = \"not prose at all, really not\";\n```\n\n| a | b |\n|---|---|\n\n---\n\nShort one.\n\n## Visiting\nBook your tickets online well before you go.\n\n- Go early in the morning to beat the crowds.\n- Bring water.\n\n[^1]: Some footnote that is long enough to count.\n";
        assert_eq!(
            texts(doc),
            vec![
                "The tower was completed in 1889 for the fair.\nIt is about 330 metres tall.",
                "Book your tickets online well before you go.",
                "- Go early in the morning to beat the crowds.\n- Bring water.",
            ]
        );
    }

    #[test]
    fn ranges_point_into_the_text() {
        let doc = "One two three four five six.\n\nSeven eight nine ten eleven twelve.";
        let ps = paragraphs(doc);
        assert_eq!(ps.len(), 2);
        assert_eq!(&doc[ps[1].clone()], "Seven eight nine ten eleven twelve.");
    }

    #[test]
    fn fingerprints_ignore_wrapping() {
        assert_eq!(fingerprint("a b\nc"), fingerprint("a  b c\n"));
        assert_ne!(fingerprint("a b c"), fingerprint("a b d"));
    }

    #[test]
    fn knows_a_finished_sentence() {
        assert!(finished("It was tall."));
        assert!(finished("Was it “tall?”"));
        assert!(finished("It was *tall*.\n"));
        assert!(!finished("It was tall and"));
    }
}
