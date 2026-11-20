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
    }
}
