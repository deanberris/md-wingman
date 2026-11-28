//! Footnotes in the preview: a reference (`[^1]`) is a marker that jumps to
//! its footnote, and the footnote ends with a way back to each place that
//! cites it.
//!
//! The Markdown view asks its plugins about every node before drawing it
//! itself, so these two claim footnote references and footnotes and draw
//! them as elements of our own.

use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::text::{
    MarkdownNode, MarkdownParseContext, MarkdownPlugin, TextView, markdown_ast,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::*;

use crate::workspace::Workspace;

/// A footnote reference or a footnote's own line, found in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// The label, lowercased: labels match whatever their case.
    pub label: String,
    /// The `[^label]` itself, without a footnote's colon.
    pub range: Range<usize>,
    /// The footnote, rather than a reference to it.
    pub definition: bool,
}

/// Every footnote mark in `text`, in order. Fenced code is skipped.
pub fn marks(text: &str) -> Vec<Mark> {
    let mut found = Vec::new();
    let mut fence: Option<&str> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let fenced = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f));
        match (fence, fenced) {
            (None, Some(f)) => fence = Some(f),
            (Some(open), Some(f)) if open == f => fence = None,
            _ => {}
        }
        if fence.is_some() || fenced.is_some() {
            continue;
        }
        marks_in_line(line, start, indent, &mut found);
    }
    found
}

fn marks_in_line(line: &str, start: usize, indent: usize, found: &mut Vec<Mark>) {
    let mut from = 0;
    while let Some(open) = line[from..].find("[^").map(|i| from + i) {
        let Some(close) = line[open..].find(']').map(|i| open + i) else {
            return;
        };
        from = close + 1;
        let label = &line[open + 2..close];
        if label.is_empty() || label.contains(char::is_whitespace) {
            continue;
        }
        found.push(Mark {
            label: label.to_lowercase(),
            range: start + open..start + close + 1,
            definition: open == indent && indent < 4 && line[close + 1..].starts_with(':'),
        });
    }
}

/// Where the footnote labelled `label` is.
pub fn definition(marks: &[Mark], label: &str) -> Option<Range<usize>> {
    let mark = marks.iter().find(|m| m.definition && m.label == label)?;
    Some(mark.range.clone())
}

/// Where the references to the footnote labelled `label` are.
pub fn citations<'a>(marks: &'a [Mark], label: &'a str) -> impl Iterator<Item = Range<usize>> + 'a {
    marks
        .iter()
        .filter(move |m| !m.definition && m.label == label)
        .map(|m| m.range.clone())
}

/// A reference's label, as the document writes it.
struct Reference(String);

/// Draws `[^1]` as a marker that jumps to its footnote.
pub struct ReferencePlugin(pub WeakEntity<Workspace>);

impl MarkdownPlugin for ReferencePlugin {
    fn name(&self) -> &str {
        "footnote-reference"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        _: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        // The parser only reports a reference whose footnote exists, so one
        // without stays plain text.
        let markdown_ast::Node::FootnoteReference(reference) = node else {
            return None;
        };
        let label = reference
            .label
            .clone()
            .unwrap_or_else(|| reference.identifier.clone());
        let text = format!("[{label}]");
        Some(MarkdownNode::new("footnote-reference", Reference(label)).text(text))
    }

    fn render(&self, node: &MarkdownNode, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let label = node.data::<Reference>().map_or("", |r| r.0.as_str());
        let key = label.to_lowercase();
        let start = node.source_range().map_or(0, |r| r.start);
        // Which of the footnote's citations this is, to name the element.
        let nth = self
            .0
            .read_with(cx, |ws, _| ws.preview.citation_index(&key, start))
            .unwrap_or(0);
        let workspace = self.0.clone();
        div()
            .id(ElementId::Name(
                format!("footnote-reference-{key}-{nth}").into(),
            ))
            .test_support()
            .cursor_pointer()
            .text_xs()
            .text_color(cx.theme().link)
            .child(SharedString::from(node.as_text().to_string()))
            .on_click(move |_, _, cx| {
                let _ = workspace.update(cx, |ws, cx| ws.go_to_footnote(&key, cx));
            })
    }
}

/// A footnote: its label and what it says, as Markdown.
struct Footnote {
    label: String,
    body: String,
}

/// Draws a footnote with a way back to each place that cites it.
pub struct FootnotePlugin(pub WeakEntity<Workspace>);

impl MarkdownPlugin for FootnotePlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "footnote"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::FootnoteDefinition(def) = node else {
            return None;
        };
        let label = def.label.clone().unwrap_or_else(|| def.identifier.clone());
        let source = cx.node_source(node).unwrap_or_default();
        let body = footnote_body(source);
        let text = format!("[{label}]: {body}");
        Some(MarkdownNode::new("footnote", Footnote { label, body }).text(text))
    }

    fn render(&self, node: &MarkdownNode, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(footnote) = node.data::<Footnote>() else {
            return div().into_any_element();
        };
        let key = footnote.label.to_lowercase();
        let cited = self
            .0
            .read_with(cx, |ws, _| ws.preview.citation_count(&key))
            .unwrap_or(0);
        let back = (0..cited).map(|nth| {
            let (workspace, key) = (self.0.clone(), key.clone());
            Button::new(ElementId::Name(format!("footnote-back-{key}-{nth}").into()))
                .ghost()
                .xsmall()
                .label("↩")
                .on_click(move |_, _, cx| {
                    let _ = workspace.update(cx, |ws, cx| ws.go_to_citation(&key, nth, cx));
                })
        });
        h_flex()
            .w_full()
            .items_start()
            .gap_2()
            .text_sm()
            .child(
                div()
                    .flex_none()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!("[{}]", footnote.label))),
            )
            .child(
                div().flex_1().min_w_0().child(
                    TextView::markdown(
                        ElementId::Name(format!("footnote-body-{key}").into()),
                        footnote.body.clone(),
                    )
                    .selectable(true),
                ),
            )
            .children(back)
            .into_any_element()
    }
}

/// What a footnote says: its source without the `[^label]:`, and without
/// the indent its later lines need to stay part of it.
fn footnote_body(source: &str) -> String {
    let after = source.split_once("]:").map_or(source, |(_, rest)| rest);
    after
        .trim_start()
        .lines()
        .map(|line| {
            let indent = line.len() - line.trim_start_matches(' ').len();
            &line[indent.min(4)..]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{citations, definition, footnote_body, marks};

    #[test]
    fn finds_references_and_footnotes() {
        let text = "One.[^a] Two.[^B]\n\n[^a]: First.\n[^b]: Second, see [^a].\n";
        let found = marks(text);
        let shown: Vec<_> = found
            .iter()
            .map(|m| (&text[m.range.clone()], m.definition))
            .collect();
        assert_eq!(
            shown,
            [
                ("[^a]", false),
                ("[^B]", false),
                ("[^a]", true),
                ("[^b]", true),
                ("[^a]", false)
            ]
        );
        assert_eq!(&text[definition(&found, "b").unwrap()], "[^b]");
        assert_eq!(citations(&found, "a").count(), 2);
        assert_eq!(definition(&found, "c"), None);
    }

    #[test]
    fn skips_fenced_code_and_things_that_are_not_labels() {
        let text = "```\n[^a]: in code\n```\n\nA [^ b] and [^] and [^a].\n";
        let found = marks(text);
        assert_eq!(found.len(), 1);
        assert!(!found[0].definition);
    }

    #[test]
    fn a_footnotes_body_loses_its_label_and_indent() {
        assert_eq!(footnote_body("[^1]: Some *text*."), "Some *text*.");
        assert_eq!(
            footnote_body("[^1]: First.\n\n    Second.\n      Third."),
            "First.\n\nSecond.\n  Third."
        );
    }
}
