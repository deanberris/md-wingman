//! Structured assistant replies and the text logic around them: choosing the
//! passage to send, and finding quoted text back in the document.

use std::ops::Range;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CopyEditReply {
    #[serde(default)]
    pub summary: String,
    pub edits: Vec<EditSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EditSuggestion {
    pub original: String,
    pub replacement: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FactCheckReply {
    #[serde(default)]
    pub summary: String,
    pub claims: Vec<Claim>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Claim {
    pub quote: String,
    pub verdict: Verdict,
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Supported,
    Disputed,
    False,
    Unverifiable,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Supported => "Supported",
            Verdict::Disputed => "Disputed",
            Verdict::False => "False",
            Verdict::Unverifiable => "Unverifiable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Source {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReferencesReply {
    #[serde(default)]
    pub summary: String,
    pub references: Vec<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Reference {
    #[serde(default)]
    pub quote: String,
    pub title: String,
    #[serde(default)]
    pub authors: String,
    #[serde(default)]
    pub year: String,
    #[serde(default)]
    pub publisher: String,
    pub url: String,
    pub relation: Relation,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Supports,
    Challenges,
    Background,
}

impl Relation {
    pub fn label(self) -> &'static str {
        match self {
            Relation::Supports => "Supports",
            Relation::Challenges => "Challenges",
            Relation::Background => "Background",
        }
    }
}

impl Reference {
    /// "Doe, J. (2020). *Title*. Publisher. <url>", leaving out what's missing.
    pub fn citation(&self) -> String {
        let mut out = String::new();
        let authors = self.authors.trim();
        let year = self.year.trim();
        match (authors.is_empty(), year.is_empty()) {
            (false, false) => out.push_str(&format!("{authors} ({year}). ")),
            (false, true) => out.push_str(&format!("{authors}. ")),
            (true, false) => out.push_str(&format!("({year}). ")),
            (true, true) => {}
        }
        out.push_str(&format!("*{}*.", self.title.trim().trim_end_matches('.')));
        let publisher = self.publisher.trim();
        if !publisher.is_empty() {
            out.push_str(&format!(" {}.", publisher.trim_end_matches('.')));
        }
        out.push_str(&format!(" <{}>", self.url.trim()));
        out
    }

    /// Short link text: the title, with the year when there is one.
    fn link_text(&self) -> String {
        let title = self.title.trim().replace(['[', ']'], "");
        match self.year.trim() {
            "" => title,
            year => format!("{title}, {year}"),
        }
    }
}

/// One change to make to the document: replace `range` with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insertion {
    pub range: Range<usize>,
    pub text: String,
}

/// Where an inline citation goes after text ending at `at`: before a final
/// full stop, so "tall." becomes "tall ([Source](url))."
fn before_sentence_end(text: &str, at: usize) -> usize {
    let head = &text[..at];
    let trimmed = head.trim_end_matches(['.', '!', '?']);
    if trimmed.len() < head.len() && head.len() - trimmed.len() == 1 {
        trimmed.len()
    } else {
        at
    }
}

/// An inline Markdown link to `reference`, placed after the text ending at `at`.
pub fn link_citation(text: &str, at: usize, reference: &Reference) -> Insertion {
    let at = before_sentence_end(text, at);
    Insertion {
        range: at..at,
        text: format!(" ([{}]({}))", reference.link_text(), reference.url.trim()),
    }
}

/// The lowest footnote number not already used in `text`.
pub fn next_footnote(text: &str) -> usize {
    let used: Vec<usize> = text
        .match_indices("[^")
        .filter_map(|(i, _)| {
            let rest = &text[i + 2..];
            let end = rest.find(']')?;
            rest[..end].parse().ok()
        })
        .collect();
    used.into_iter().max().unwrap_or(0) + 1
}

/// A footnote marker after the text ending at `at`, and its definition at
/// the end of the document. Apply them in the order returned (last
/// position first), so the first insertion doesn't shift the second.
pub fn footnote_citation(text: &str, at: usize, reference: &Reference) -> (usize, [Insertion; 2]) {
    let n = next_footnote(text);
    let separator = if text.is_empty() || text.ends_with("\n\n") {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    // Keep footnote definitions together when the document already ends with one.
    let last_line = text.trim_end().lines().last().unwrap_or("");
    let separator = if last_line.starts_with("[^") {
        if text.ends_with('\n') {
            ""
        } else {
            "\n"
        }
    } else {
        separator
    };
    let end = text.len();
    (
        n,
        [
            Insertion {
                range: end..end,
                text: format!("{separator}[^{n}]: {}\n", reference.citation()),
            },
            Insertion {
                range: at..at,
                text: format!("[^{n}]"),
            },
        ],
    )
}

/// The passage to send: the selection, or if nothing is selected, the
/// paragraph around the cursor (the text between blank lines).
pub fn passage_range(text: &str, selection: Range<usize>) -> Range<usize> {
    if !selection.is_empty() {
        return selection;
    }
    let cursor = selection.start.min(text.len());
    // A cursor on a blank line belongs to the paragraph after it.
    let look_back = if text[cursor..].starts_with('\n') {
        cursor + 1
    } else {
        cursor
    };
    let start = text[..look_back].rfind("\n\n").map(|i| i + 2).unwrap_or(0);
    let end = text[start..]
        .trim_start_matches('\n')
        .find("\n\n")
        .map(|i| text.len() - text[start..].trim_start_matches('\n').len() + i)
        .unwrap_or(text.len());
    // Trim surrounding newlines so the range is the paragraph itself.
    let slice = &text[start..end];
    let lead = slice.len() - slice.trim_start_matches('\n').len();
    let trail = slice.len() - slice.trim_end_matches('\n').len();
    start + lead..end - trail
}

/// Finds each quote in `text`, preferring a match inside `within` and then
/// the match closest to it, and returns their byte ranges. Locating them
/// together means that repeated short quotes (for example two
/// edits of "it") land on different matches rather than the same one.
pub fn locate_all<'a>(
    text: &str,
    quotes: impl IntoIterator<Item = &'a str>,
    within: &Range<usize>,
) -> Vec<Option<Range<usize>>> {
    let mut taken: Vec<Range<usize>> = Vec::new();
    quotes
        .into_iter()
        .map(|quote| {
            let found = candidates(text, quote, within)
                .into_iter()
                .find(|r| !taken.iter().any(|t| t.start < r.end && r.start < t.end))?;
            taken.push(found.clone());
            Some(found)
        })
        .collect()
}

/// Every match of `quote`, best first: inside `within`, then by distance.
fn candidates(text: &str, quote: &str, within: &Range<usize>) -> Vec<Range<usize>> {
    if quote.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<Range<usize>> = text
        .match_indices(quote)
        .map(|(i, _)| i..i + quote.len())
        .collect();
    found.sort_by_key(|r| {
        let inside = r.start >= within.start && r.end <= within.end;
        let distance = if r.start < within.start {
            within.start - r.start
        } else {
            r.start.saturating_sub(within.start)
        };
        (!inside, distance)
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locate(text: &str, quote: &str, within: &Range<usize>) -> Option<Range<usize>> {
        locate_all(text, [quote], within).pop().flatten()
    }

    const DOC: &str =
        "# Title\n\nFirst para has teh typo.\nStill first.\n\nSecond para, teh end.\n";

    #[test]
    fn selection_wins() {
        assert_eq!(passage_range(DOC, 2..5), 2..5);
    }

    #[test]
    fn empty_selection_expands_to_paragraph() {
        let cursor = DOC.find("typo").unwrap();
        let r = passage_range(DOC, cursor..cursor);
        assert_eq!(&DOC[r], "First para has teh typo.\nStill first.");
        let cursor = DOC.find("end").unwrap();
        let r = passage_range(DOC, cursor..cursor);
        assert_eq!(&DOC[r], "Second para, teh end.");
        let r = passage_range(DOC, 0..0);
        assert_eq!(&DOC[r], "# Title");
        // On the blank line after the title: the next paragraph only.
        let blank = DOC.find("\n\n").unwrap() + 1;
        let r = passage_range(DOC, blank..blank);
        assert_eq!(&DOC[r], "First para has teh typo.\nStill first.");
        // At the end of a paragraph's last line: that paragraph.
        let eol = DOC.find("first.").unwrap() + "first.".len();
        let r = passage_range(DOC, eol..eol);
        assert_eq!(&DOC[r], "First para has teh typo.\nStill first.");
    }

    #[test]
    fn locate_prefers_the_passage() {
        let second = DOC.find("Second").unwrap()..DOC.len();
        let r = locate(DOC, "teh", &second).unwrap();
        assert!(r.start > second.start);
        let first = passage_range(DOC, 12..12);
        let r = locate(DOC, "teh", &first).unwrap();
        assert!(r.end <= first.end);
        assert_eq!(locate(DOC, "missing", &first), None);
    }

    #[test]
    fn locate_all_gives_repeats_distinct_ranges() {
        let text = "it is what it is";
        let ranges = locate_all(text, ["it", "it"], &(0..text.len()));
        assert_eq!(ranges, vec![Some(0..2), Some(11..13)]);
    }

    fn reference(year: &str) -> Reference {
        Reference {
            quote: String::new(),
            title: "A Study".into(),
            authors: "Doe, J.".into(),
            year: year.into(),
            publisher: "Journal".into(),
            url: "https://x.org/a".into(),
            relation: Relation::Supports,
            note: String::new(),
        }
    }

    fn apply(text: &str, edits: &[Insertion]) -> String {
        let mut out = text.to_string();
        for e in edits {
            out.replace_range(e.range.clone(), &e.text);
        }
        out
    }

    #[test]
    fn citation_formats() {
        assert_eq!(
            reference("2020").citation(),
            "Doe, J. (2020). *A Study*. Journal. <https://x.org/a>"
        );
        let mut r = reference("");
        r.authors.clear();
        r.publisher.clear();
        assert_eq!(r.citation(), "*A Study*. <https://x.org/a>");
    }

    #[test]
    fn link_goes_before_the_full_stop() {
        let text = "It is tall. More.";
        let at = text.find(" More").unwrap();
        let ins = link_citation(text, at, &reference("2020"));
        assert_eq!(
            apply(text, &[ins]),
            "It is tall ([A Study, 2020](https://x.org/a)). More."
        );
    }

    #[test]
    fn footnotes_number_and_collect_at_the_end() {
        assert_eq!(next_footnote("no notes"), 1);
        assert_eq!(next_footnote("a[^1] b[^3] c[^note]"), 4);

        let text = "It is tall.\n";
        let (n, edits) = footnote_citation(text, 11, &reference("2020"));
        assert_eq!(n, 1);
        let once = apply(text, &edits);
        assert_eq!(
            once,
            "It is tall.[^1]\n\n[^1]: Doe, J. (2020). *A Study*. Journal. <https://x.org/a>\n"
        );
        let (n, edits) = footnote_citation(&once, 11, &reference("2021"));
        assert_eq!(n, 2);
        let twice = apply(&once, &edits);
        assert!(twice.starts_with("It is tall.[^2][^1]\n\n[^1]: "));
        assert!(twice.ends_with(
            "<https://x.org/a>\n[^2]: Doe, J. (2021). *A Study*. Journal. <https://x.org/a>\n"
        ));
    }

    #[test]
    fn parses_service_replies() {
        let ce: CopyEditReply = serde_json::from_str(
            r#"{"summary":"s","edits":[{"original":"teh","replacement":"the","reason":"typo"}]}"#,
        )
        .unwrap();
        assert_eq!(ce.edits[0].replacement, "the");
        let fc: FactCheckReply = serde_json::from_str(
            r#"{"summary":"s","claims":[{"quote":"q","verdict":"false","explanation":"e","sources":[{"title":"t","url":"https://x"}]}]}"#,
        )
        .unwrap();
        assert_eq!(fc.claims[0].verdict, Verdict::False);
        let refs: ReferencesReply = serde_json::from_str(
            r#"{"summary":"s","references":[{"quote":"q","title":"t","authors":"","year":"","publisher":"p","url":"https://x","relation":"challenges","note":"n"}]}"#,
        )
        .unwrap();
        assert_eq!(refs.references[0].relation, Relation::Challenges);
    }
}
