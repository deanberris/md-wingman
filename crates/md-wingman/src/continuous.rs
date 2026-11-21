//! Continuous mode: when typing (or a dictation burst) pauses, check the
//! paragraphs that changed, one at a time, and show what's found as it comes.
//!
//! Each paragraph's text is fingerprinted once it has been checked, so only
//! new or changed paragraphs cost a request. When the mode is turned on, or a
//! document is opened, the paragraphs already there count as checked: it
//! follows what you write, and the sidebar buttons check the rest on demand.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::*;
use wingman_client::TaskKind;

use crate::jev::{Grader, Grades, JevError};
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
    /// Jev's grades, by paragraph fingerprint.
    grades: HashMap<u64, Grades>,
    /// The paragraph Jev is grading.
    grading: Option<u64>,
    _grading_task: Option<Task<()>>,
    /// The TypeSafe key, once read from the Keychain.
    jev_key: Option<Option<String>>,
    /// Set when Jev can't be used until the key changes.
    jev_blocked: bool,
    /// After Jev says it's busy: wait until then, backing off each time.
    retry_at: Option<Instant>,
    backoff: Duration,
    /// The latest grades, to show in the sidebar.
    last_grade: Option<GradeReport>,
}

/// Jev's grades for the paragraph it graded last.
#[derive(Debug, Clone, PartialEq)]
pub struct GradeReport {
    /// The paragraph's first few words.
    pub excerpt: String,
    /// Its fingerprint, to match the checks that follow.
    pub fingerprint: u64,
    pub grades: Grades,
    /// The checks the grades started, and how each one went.
    pub checks: Vec<(TaskKind, CheckStatus)>,
}

/// How a check started by a grade is going.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckStatus {
    Running,
    /// Finished, with this many suggestions or claims.
    Done(usize),
    Failed(String),
}

impl GradeReport {
    pub fn status(&self, kind: TaskKind) -> Option<&CheckStatus> {
        self.checks.iter().find(|(k, _)| *k == kind).map(|(_, s)| s)
    }

    fn set(&mut self, kind: TaskKind, status: CheckStatus) {
        match self.checks.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, s)) => *s = status,
            None => self.checks.push((kind, status)),
        }
    }
}

/// What continuous mode does with one kind of help after Jev's grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Above the threshold: Claude checks it.
    Check,
    /// At or below the threshold: no request.
    Skip,
    /// Turned off for checking as you write.
    Off,
}

/// Each kind of help with its score and what happens to it.
pub fn verdicts(
    grades: &Grades,
    kinds: &[TaskKind],
    threshold: f32,
) -> Vec<(TaskKind, f32, Verdict)> {
    [
        TaskKind::CopyEdit,
        TaskKind::FactCheck,
        TaskKind::FindReferences,
    ]
    .into_iter()
    .map(|k| {
        let score = grade_for(grades, k);
        let verdict = if !kinds.contains(&k) {
            Verdict::Off
        } else if score > threshold {
            Verdict::Check
        } else {
            Verdict::Skip
        };
        (k, score, verdict)
    })
    .collect()
}

fn excerpt(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = words.iter().take(8).copied().collect::<Vec<_>>().join(" ");
    if words.len() > 8 {
        out.push('…');
    }
    out
}

impl Continuous {
    pub fn running_id(&self) -> Option<usize> {
        self.running.as_ref().map(|(id, _)| *id)
    }

    pub fn is_checking(&self) -> bool {
        self.running.is_some() || self.grading.is_some()
    }

    pub fn is_grading(&self) -> bool {
        self.grading.is_some()
    }

    pub fn last_grade(&self) -> Option<&GradeReport> {
        self.last_grade.as_ref()
    }

    /// The TypeSafe key changed: use it from now on.
    pub fn key_changed(&mut self, key: Option<String>) {
        self.jev_key = Some(key);
        self.jev_blocked = false;
        self.retry_at = None;
        self.backoff = Duration::ZERO;
    }

    /// The key, reading the Keychain only if it hasn't been read yet.
    pub(crate) fn key(&mut self, grader: &dyn Grader) -> Result<Option<String>, String> {
        if self.jev_key.is_none() {
            self.jev_key = Some(grader.load_key()?);
        }
        Ok(self.jev_key.clone().flatten())
    }

    /// The key as read so far: `None` until the Keychain has been read.
    /// Grading never reads the Keychain itself, so a prompt can't appear
    /// while you write.
    pub(crate) fn cached_key(&self) -> Option<Option<String>> {
        self.jev_key.clone()
    }

    /// Grading waits until a key is set.
    pub fn block_until_key(&mut self) {
        self.jev_blocked = true;
    }

    fn mark(&mut self, kind: TaskKind, text: &str) {
        self.checked.insert((kind_key(kind), fingerprint(text)));
    }

    fn is_checked(&self, kind: TaskKind, text: &str) -> bool {
        self.checked.contains(&(kind_key(kind), fingerprint(text)))
    }

    /// Counts every paragraph in `text` as checked.
    fn baseline(&mut self, text: &str, kinds: &[TaskKind]) {
        self.last_grade = None;
        for range in paragraphs(text) {
            for &kind in kinds {
                self.mark(kind, &text[range.clone()]);
            }
        }
    }
}

fn grade_for(grades: &Grades, kind: TaskKind) -> f32 {
    match kind {
        TaskKind::CopyEdit => grades.copy_edit,
        TaskKind::FactCheck => grades.fact_check,
        TaskKind::FindReferences => grades.references,
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
        // References only with the pre-filter: they're worth a request only
        // for paragraphs that make citable claims.
        if self.settings.jev_prefilter {
            kinds.push(TaskKind::FindReferences);
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
        self.continuous.last_edit = Some(cx.background_executor().now());
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
        self.schedule_continuous_at(self.delay(), cx);
    }

    fn schedule_continuous_at(&mut self, delay: Duration, cx: &mut Context<Self>) {
        self.continuous.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| this.continuous_tick(cx));
        }));
    }

    /// Starts the next check, if typing has paused and nothing is running.
    pub(crate) fn continuous_tick(&mut self, cx: &mut Context<Self>) {
        // The executor's clock, which tests can move forward.
        let now = cx.background_executor().now();
        if !self.settings.continuous
            || self.assistant.is_none()
            || self.continuous.is_checking()
            || self
                .continuous
                .last_edit
                .is_some_and(|t| now.saturating_duration_since(t) < self.delay())
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
        if self.settings.jev_prefilter {
            if self.continuous.jev_blocked {
                return;
            }
            if let Some(at) = self.continuous.retry_at {
                if at > now {
                    self.schedule_continuous_at(at - now, cx);
                    return;
                }
            }
        }
        let threshold = self.settings.jev_threshold;
        let mut next = None;
        for p in candidates {
            let paragraph = &text[p.clone()];
            let pending: Vec<TaskKind> = kinds
                .iter()
                .copied()
                .filter(|&k| !self.continuous.is_checked(k, paragraph))
                .collect();
            if pending.is_empty() {
                continue;
            }
            if !self.settings.jev_prefilter {
                next = Some((pending[0], p));
                break;
            }
            // Grade it first, then act only on what scores above the bar.
            let fp = fingerprint(paragraph);
            let Some(grades) = self.continuous.grades.get(&fp).copied() else {
                self.start_grading(fp, paragraph.to_string(), cx);
                return;
            };
            for k in pending {
                if grade_for(&grades, k) > threshold {
                    next = Some((k, p.clone()));
                    break;
                }
                self.continuous.mark(k, paragraph);
            }
            if next.is_some() {
                break;
            }
        }
        let Some((kind, passage)) = next else {
            return;
        };
        let sent = text[passage.clone()].to_string();
        self.continuous.mark(kind, &sent);
        let status = match self.start_run_on(kind, passage, true, cx) {
            Some(id) => {
                self.continuous.running = Some((id, sent.clone()));
                CheckStatus::Running
            }
            None => {
                // It didn't start (the assistant said why): try again after
                // the next edit rather than count it as checked.
                self.continuous
                    .checked
                    .remove(&(kind_key(kind), fingerprint(&sent)));
                CheckStatus::Failed(
                    self.notice
                        .as_ref()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "it couldn't start".into()),
                )
            }
        };
        self.continuous_report(&sent, kind, status);
        cx.notify();
    }

    /// Notes how a check of `paragraph` is going, if it's the one Jev
    /// graded last.
    fn continuous_report(&mut self, paragraph: &str, kind: TaskKind, status: CheckStatus) {
        let fp = fingerprint(paragraph);
        if let Some(report) = self
            .continuous
            .last_grade
            .as_mut()
            .filter(|r| r.fingerprint == fp)
        {
            report.set(kind, status);
        }
    }

    /// Called when a background run ends, however it ended, with how many
    /// findings it made.
    pub(crate) fn continuous_run_ended(
        &mut self,
        id: usize,
        kind: TaskKind,
        failure: Option<String>,
        found: usize,
        cx: &mut Context<Self>,
    ) {
        let Some((running, sent)) = self.continuous.running.take() else {
            return;
        };
        if running != id {
            self.continuous.running = Some((running, sent));
            return;
        }
        let status = match &failure {
            None => CheckStatus::Done(found),
            Some(m) if m == "cancelled" => {
                CheckStatus::Failed("stopped: the paragraph changed".into())
            }
            Some(m) => CheckStatus::Failed(m.clone()),
        };
        self.continuous_report(&sent, kind, status);
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
        for kind in [
            TaskKind::CopyEdit,
            TaskKind::FactCheck,
            TaskKind::FindReferences,
        ] {
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

impl Workspace {
    fn start_grading(&mut self, fp: u64, paragraph: String, cx: &mut Context<Self>) {
        let key = match self.continuous.cached_key() {
            Some(Some(key)) => key,
            // Still reading the Keychain: grade once it's done.
            None => return,
            Some(None) => {
                self.continuous.jev_blocked = true;
                self.set_notice(
                    "The Jev pre-filter needs a TypeSafe API key: Assistant ▸ Set TypeSafe API Key…",
                    cx,
                );
                return;
            }
        };
        let model = self.settings.jev_model.clone();
        let words = excerpt(&paragraph);
        let grader = self.grader.clone();
        self.continuous.grading = Some(fp);
        let grading = cx
            .background_executor()
            .spawn(async move { grader.grade(&key, &model, &paragraph) });
        self.continuous._grading_task = Some(cx.spawn(async move |this, cx| {
            let result = grading.await;
            let _ = this.update(cx, |this, cx| this.grading_done(fp, words, result, cx));
        }));
        cx.notify();
    }

    fn grading_done(
        &mut self,
        fp: u64,
        excerpt: String,
        result: Result<Grades, JevError>,
        cx: &mut Context<Self>,
    ) {
        self.continuous.grading = None;
        self.continuous._grading_task = None;
        match result {
            Ok(grades) => {
                self.continuous.grades.insert(fp, grades);
                self.continuous.backoff = Duration::ZERO;
                self.continuous.retry_at = None;
                self.continuous.last_grade = Some(GradeReport {
                    excerpt,
                    fingerprint: fp,
                    grades,
                    checks: Vec::new(),
                });
                self.continuous_tick(cx);
            }
            Err(JevError::BadKey) => {
                self.continuous.jev_blocked = true;
                self.set_notice(
                    "TypeSafe didn't accept the API key. Set it again: Assistant ▸ Set TypeSafe API Key…",
                    cx,
                );
            }
            Err(e) => {
                let backoff = (self.continuous.backoff * 2)
                    .clamp(Duration::from_secs(5), Duration::from_secs(120));
                let wait = match &e {
                    JevError::Busy {
                        retry_after: Some(secs),
                    } => Duration::from_secs(*secs).max(backoff),
                    _ => backoff,
                };
                self.continuous.backoff = backoff;
                self.continuous.retry_at = Some(cx.background_executor().now() + wait);
                self.set_notice(format!("{e}; trying again in {}s", wait.as_secs()), cx);
                self.schedule_continuous_at(wait, cx);
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, finished, paragraphs, verdicts, Verdict};
    use crate::jev::Grades;
    use wingman_client::TaskKind;

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

    #[test]
    fn verdicts_follow_the_threshold() {
        let grades = Grades {
            copy_edit: 0.8,
            fact_check: 0.66,
            references: 0.9,
        };
        let kinds = [TaskKind::CopyEdit, TaskKind::FactCheck];
        let got: Vec<Verdict> = verdicts(&grades, &kinds, 0.66)
            .into_iter()
            .map(|(_, _, v)| v)
            .collect();
        assert_eq!(got, vec![Verdict::Check, Verdict::Skip, Verdict::Off]);
    }
}
