//! Findings: what the assistant found in the text (suggested edits, checked
//! claims, references), each tied to the words it quotes.
//!
//! A finding follows its text. Its highlight moves as the document is
//! edited; when the text is deleted or rewritten the finding is hidden, and
//! when the text comes back (undo, redo, or pasting it back) the finding
//! comes back with it. Accepting an edit hides it the same way, so undoing
//! the accept brings the suggestion back too.

use std::ops::Range;

use gpui_kit::component::input::{TextDecoration, TextDecorationCollection};
use gpui_kit::*;

use crate::store::{self, SavedClaim};
use crate::suggestions::{
    combine_edits, footnote_citation, link_citation, relocate, Claim, EditSuggestion, Insertion,
    Reference, Relation, Verdict,
};
use crate::workspace::{ViewMode, Workspace};

/// How far (in bytes) to look for a finding's text around where it was last
/// seen. Far enough to follow edits above it, near enough not to latch on to
/// the same words in another paragraph.
const REACH: usize = 2_000;

/// How far from an edit's anchor its replacement may sit for a redo to count
/// as accepting it again. Earlier edits in the same change shift it a little.
const REDO_REACH: usize = 64;

pub enum FindingKind {
    Edit(EditSuggestion),
    Claim(Claim),
    Reference(Reference),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingState {
    Open,
    /// The edit was applied or the reference inserted.
    Accepted,
    Dismissed,
}

pub struct Finding {
    pub id: u64,
    pub kind: FindingKind,
    /// The text this finding is about, exactly as it appears in the document.
    pub quote: String,
    style: HighlightStyle,
    /// Where the quote starts, or last started.
    pub anchor: usize,
    /// Whether the quoted text is in the document now.
    pub live: bool,
    pub state: FindingState,
    /// The run that produced it, while that run's card is open.
    pub run: Option<usize>,
    /// For claims: when they were checked (seconds since the Unix epoch).
    pub checked_at: u64,
    highlight: Option<TextDecorationCollection>,
}

impl Finding {
    pub fn new(
        id: u64,
        kind: FindingKind,
        quote: String,
        anchor: usize,
        run: Option<usize>,
    ) -> Self {
        let style = match &kind {
            FindingKind::Edit(_) => edit_style(),
            FindingKind::Claim(c) => claim_style(c.verdict),
            FindingKind::Reference(_) => reference_style(),
        };
        Finding {
            id,
            kind,
            quote,
            style,
            anchor,
            live: false,
            state: FindingState::Open,
            run,
            checked_at: store::now(),
            highlight: None,
        }
    }

    /// Shown in the sidebar and highlighted in the text.
    pub fn visible(&self) -> bool {
        self.live && self.state == FindingState::Open
    }

    pub fn is_claim(&self) -> bool {
        matches!(self.kind, FindingKind::Claim(_))
    }

    /// For a reference whose quote couldn't be found: anchor it to text it
    /// should be cited after instead, without highlighting it.
    pub fn quiet(mut self) -> Self {
        self.style = HighlightStyle::default();
        self
    }
}

pub fn edit_style() -> HighlightStyle {
    HighlightStyle {
        underline: Some(UnderlineStyle {
            thickness: px(2.),
            color: Some(hsla(0.08, 0.9, 0.5, 1.)),
            wavy: true,
        }),
        ..Default::default()
    }
}

pub fn verdict_color(verdict: Verdict) -> Hsla {
    match verdict {
        Verdict::Supported => hsla(0.38, 0.6, 0.4, 1.),
        Verdict::Disputed => hsla(0.11, 0.9, 0.45, 1.),
        Verdict::False => hsla(0.0, 0.75, 0.5, 1.),
        Verdict::Unverifiable => hsla(0.0, 0.0, 0.5, 1.),
    }
}

pub fn relation_color(relation: Relation) -> Hsla {
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
    pub(crate) fn add_finding(
        &mut self,
        kind: FindingKind,
        quote: String,
        anchor: usize,
        run: Option<usize>,
        quiet: bool,
    ) -> u64 {
        let id = self.next_finding_id;
        self.next_finding_id += 1;
        let finding = Finding::new(id, kind, quote, anchor, run);
        self.findings
            .push(if quiet { finding.quiet() } else { finding });
        id
    }

    pub(crate) fn finding(&self, id: u64) -> Option<&Finding> {
        self.findings.iter().find(|f| f.id == id)
    }

    fn finding_mut(&mut self, id: u64) -> Option<&mut Finding> {
        self.findings.iter_mut().find(|f| f.id == id)
    }

    /// Re-attaches every finding to its text after the document changed:
    /// follows moved text, hides findings whose text is gone, and brings
    /// back those whose text has returned.
    pub(crate) fn reconcile_findings(&mut self, cx: &mut Context<Self>) {
        if self.findings.is_empty() {
            return;
        }
        let text = self.text(cx);
        let editor = self.editor().clone();
        let mut claims_changed = false;
        for f in &mut self.findings {
            // Dismissed findings stay gone, and so do inserted references.
            if f.state == FindingState::Dismissed
                || f.state == FindingState::Accepted && !matches!(f.kind, FindingKind::Edit(_))
            {
                continue;
            }
            // Still where its highlight says, with the same words?
            if let Some(range) = f
                .highlight
                .as_ref()
                .and_then(|h| h.get_ranges(cx).into_iter().next())
            {
                if text.get(range.clone()) == Some(f.quote.as_str()) {
                    if f.anchor != range.start {
                        f.anchor = range.start;
                        claims_changed |= f.is_claim();
                    }
                    continue;
                }
            }
            if let Some(h) = f.highlight.take() {
                h.clear(cx);
            }

            // An accepted edit comes back as a suggestion if its original
            // text is back exactly where it was: the accept was undone.
            if f.state == FindingState::Accepted {
                let restored = matches!(f.kind, FindingKind::Edit(_))
                    && text.get(f.anchor..f.anchor + f.quote.len()) == Some(f.quote.as_str());
                if !restored {
                    f.live = false;
                    continue;
                }
                f.state = FindingState::Open;
            }

            let was_live = f.live;
            match relocate(&text, &f.quote, f.anchor, REACH) {
                Some(range) => {
                    f.anchor = range.start;
                    f.live = true;
                    let style = f.style;
                    f.highlight = Some(editor.update(cx, |state, cx| {
                        state.create_decorations_collection(
                            vec![TextDecoration::new(range, style)],
                            cx,
                        )
                    }));
                }
                None => {
                    f.live = false;
                    // Its fix is where it was: the accept was redone.
                    if let FindingKind::Edit(e) = &f.kind {
                        if relocate(&text, &e.replacement, f.anchor, REDO_REACH).is_some() {
                            f.state = FindingState::Accepted;
                        }
                    }
                }
            }
            claims_changed |= f.is_claim() && (was_live != f.live);
        }
        if claims_changed {
            self.save_claims(cx);
        }
        cx.notify();
    }

    /// The finding's current range in the document, if its text is there.
    pub(crate) fn finding_range(&self, id: u64, cx: &App) -> Option<Range<usize>> {
        let f = self.finding(id)?;
        let range = f.highlight.as_ref()?.get_ranges(cx).into_iter().next()?;
        let text = self.editor().read(cx).value();
        (text.get(range.clone()) == Some(f.quote.as_str())).then_some(range)
    }

    pub(crate) fn dismiss_finding(&mut self, id: u64, cx: &mut Context<Self>) {
        let mut was_claim = false;
        if let Some(f) = self.finding_mut(id) {
            f.state = FindingState::Dismissed;
            was_claim = f.is_claim();
            if let Some(h) = f.highlight.take() {
                h.clear(cx);
            }
        }
        if was_claim {
            self.save_claims(cx);
        }
        cx.notify();
    }

    /// Removes findings for good, along with their highlights.
    pub(crate) fn drop_findings(&mut self, ids: &[u64], cx: &mut Context<Self>) {
        let mut dropped_claim = false;
        self.findings.retain_mut(|f| {
            if !ids.contains(&f.id) {
                return true;
            }
            dropped_claim |= f.is_claim();
            if let Some(h) = f.highlight.take() {
                h.clear(cx);
            }
            false
        });
        if dropped_claim {
            self.save_claims(cx);
        }
    }

    /// Applies edits to the document as one undoable change.
    pub(crate) fn apply_edits(
        &mut self,
        edits: &[Insertion],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let text = self.text(cx);
        let Some((span, replacement)) = combine_edits(&text, edits) else {
            return false;
        };
        if self.mode == ViewMode::Preview {
            self.set_mode(ViewMode::Raw, window, cx);
        }
        let end = span.start + replacement.len();
        self.editor().update(cx, |state, cx| {
            state.set_selected_range(span, cx);
            state.replace(replacement, window, cx);
            state.set_selected_range(end..end, cx);
        });
        true
    }

    /// Accepts suggested edits, all as one undoable change.
    pub(crate) fn accept_edits(
        &mut self,
        ids: &[u64],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut edits = Vec::new();
        let mut accepted = Vec::new();
        for &id in ids {
            let Some(range) = self.finding_range(id, cx) else {
                continue;
            };
            if let Some(FindingKind::Edit(e)) = self.finding(id).map(|f| &f.kind) {
                edits.push(Insertion {
                    range,
                    text: e.replacement.clone(),
                });
                accepted.push(id);
            }
        }
        if edits.is_empty() {
            self.reconcile_findings(cx);
            return;
        }
        // Mark them first, so reconciling after the edit sees them accepted.
        for &id in &accepted {
            if let Some(f) = self.finding_mut(id) {
                f.state = FindingState::Accepted;
                if let Some(h) = f.highlight.take() {
                    h.clear(cx);
                }
            }
        }
        if !self.apply_edits(&edits, window, cx) {
            for &id in &accepted {
                if let Some(f) = self.finding_mut(id) {
                    f.state = FindingState::Open;
                }
            }
            self.set_notice("Those edits overlap; accept them one at a time", cx);
            self.reconcile_findings(cx);
        }
    }

    /// Cites a reference after the claim it backs, as a footnote or an
    /// inline link, as one undoable change.
    pub(crate) fn insert_reference(
        &mut self,
        id: u64,
        footnote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = self.finding_range(id, cx) else {
            self.reconcile_findings(cx);
            return;
        };
        let Some(FindingKind::Reference(reference)) = self.finding(id).map(|f| &f.kind) else {
            return;
        };
        let text = self.text(cx);
        let (edits, notice): (Vec<Insertion>, String) = if footnote {
            let (n, edits) = footnote_citation(&text, range.end, reference);
            (edits.into(), format!("Added footnote [^{n}]"))
        } else {
            (
                vec![link_citation(&text, range.end, reference)],
                "Added a link".into(),
            )
        };
        if let Some(f) = self.finding_mut(id) {
            f.state = FindingState::Accepted;
            if let Some(h) = f.highlight.take() {
                h.clear(cx);
            }
        }
        if self.apply_edits(&edits, window, cx) {
            self.set_notice(notice, cx);
        }
    }

    pub(crate) fn reveal_finding(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.finding_range(id, cx) else {
            self.reconcile_findings(cx);
            return;
        };
        if self.mode == ViewMode::Preview {
            self.set_mode(ViewMode::Raw, window, cx);
        }
        self.editor().update(cx, |state, cx| {
            state.set_selected_range(range, cx);
            state.focus(window, cx);
        });
    }

    /// The live claims under the cursor or selection.
    pub(crate) fn claims_at_cursor(&self, cx: &App) -> Vec<u64> {
        let selection = self.editor().read(cx).selected_range();
        self.findings
            .iter()
            .filter(|f| f.is_claim() && f.visible())
            .filter(|f| {
                let r = f.anchor..f.anchor + f.quote.len();
                r.start <= selection.end && selection.start <= r.end
            })
            .map(|f| f.id)
            .collect()
    }

    /// Saves the document's fact-checks, if it has a file.
    pub(crate) fn save_claims(&mut self, _cx: &mut Context<Self>) {
        let Some(path) = self.document_path() else {
            return;
        };
        let claims = self
            .findings
            .iter()
            .filter(|f| f.state != FindingState::Dismissed)
            .filter_map(|f| match &f.kind {
                FindingKind::Claim(c) => Some(SavedClaim {
                    claim: c.clone(),
                    anchor: f.anchor,
                    checked_at: f.checked_at,
                }),
                _ => None,
            })
            .collect();
        if let Err(e) = store::save(&path, claims) {
            self.notice = Some(format!("Couldn't save fact-checks: {e}").into());
        }
    }

    /// Restores a document's saved fact-checks after opening it.
    pub(crate) fn load_claims(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.findings.iter().map(|f| f.id).collect();
        self.drop_findings(&ids, cx);
        let Some(path) = self.document_path() else {
            return;
        };
        for saved in store::load(&path) {
            let quote = saved.claim.quote.clone();
            let id = self.add_finding(
                FindingKind::Claim(saved.claim),
                quote,
                saved.anchor,
                None,
                false,
            );
            if let Some(f) = self.finding_mut(id) {
                f.checked_at = saved.checked_at;
            }
        }
        self.reconcile_findings(cx);
    }
}
