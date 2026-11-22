//! Formatting the selection with Markdown: bold, italic, strikethrough and
//! code around the selected text, links, and line styles (headings, quotes,
//! lists, code blocks) on the lines it touches. Each style toggles: applying
//! it where it's already there takes it off.
//!
//! The text work is plain functions over the document and the selection, so
//! it's tested without a window. `Workspace` applies the result as one edit,
//! which is one undo step.

use std::ops::Range;

use gpui_kit::*;

use crate::workspace::{ViewMode, Workspace};
use crate::{
    Bold, BulletedList, CodeBlock, Heading1, Heading2, Heading3, InlineCode, InsertLink, Italic,
    NumberedList, Quote, Strikethrough,
};

/// A change to the document: replace `range` with `text`, then select
/// `select`, which is in the changed document's offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
    pub select: Range<usize>,
}

/// Styles that wrap the selected text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inline {
    Bold,
    Italic,
    Strikethrough,
    Code,
}

impl Inline {
    fn marker(self) -> &'static str {
        match self {
            Inline::Bold => "**",
            // Underscores, so italic never reads as half of a bold marker.
            Inline::Italic => "_",
            Inline::Strikethrough => "~~",
            Inline::Code => "`",
        }
    }
}

/// Styles that apply to whole lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    Heading(usize),
    Quote,
    Bulleted,
    Numbered,
}

/// The selection with surrounding spaces left out: Markdown emphasis can't
/// start or end with a space, and a double-click often takes one.
fn trim_range(text: &str, range: Range<usize>) -> Range<usize> {
    let inner = &text[range.clone()];
    let start = range.start + (inner.len() - inner.trim_start().len());
    let end = range.end - (inner.len() - inner.trim_end().len());
    if start >= end {
        return range.start..range.start;
    }
    start..end
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '’' || c == '-'
}

/// The word around an empty selection, if the cursor touches one.
fn word_at(text: &str, cursor: usize) -> Option<Range<usize>> {
    let start = text[..cursor]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(cursor, |(i, _)| i);
    let end = text[cursor..]
        .char_indices()
        .find(|(_, c)| !is_word_char(*c))
        .map_or(text.len(), |(i, _)| cursor + i);
    (start < end).then_some(start..end)
}

/// The selection to style: trimmed, or the word under the cursor when
/// nothing is selected. Empty when there's no word there either.
fn target(text: &str, selection: Range<usize>) -> Range<usize> {
    let selection = clamp(text, selection);
    if selection.is_empty() {
        return word_at(text, selection.start).unwrap_or(selection);
    }
    trim_range(text, selection)
}

fn clamp(text: &str, selection: Range<usize>) -> Range<usize> {
    let end = selection.end.min(text.len());
    selection.start.min(end)..end
}

/// Wraps the selection in `style`'s markers, or takes them off when the
/// selection already has them, inside or just outside it. A selection that
/// spans lines gets a code block rather than inline code.
pub fn toggle_inline(text: &str, selection: Range<usize>, style: Inline) -> Edit {
    let range = target(text, selection);
    if style == Inline::Code && text[range.clone()].contains('\n') {
        return toggle_code_block(text, range);
    }
    let marker = style.marker();
    let m = marker.len();
    let inner = &text[range.clone()];

    // Selected along with its markers: **bold**
    if inner.len() >= 2 * m && inner.starts_with(marker) && inner.ends_with(marker) {
        let unwrapped = inner[m..inner.len() - m].to_string();
        let end = range.start + unwrapped.len();
        return Edit {
            select: range.start..end,
            range,
            text: unwrapped,
        };
    }
    // Selected inside its markers, maybe with other styles between:
    // **[bold]** or **_[bold italic]_**
    if let Some(outer) = markers_around(text, range.clone(), marker) {
        let unwrapped = text[outer.start + m..outer.end - m].to_string();
        let start = range.start - m;
        return Edit {
            range: outer,
            text: unwrapped,
            select: start..start + inner.len(),
        };
    }
    // Nothing to wrap: put the markers down with the cursor between them.
    if range.is_empty() {
        let at = range.start + m;
        return Edit {
            range,
            text: format!("{marker}{marker}"),
            select: at..at,
        };
    }
    let start = range.start + m;
    Edit {
        text: format!("{marker}{inner}{marker}"),
        select: start..start + inner.len(),
        range,
    }
}

const MARKERS: [&str; 4] = ["**", "~~", "_", "`"];

/// Where `marker` wraps `range`, looking out past other styles' markers in
/// matching pairs: the range from the opening marker to the closing one.
fn markers_around(text: &str, range: Range<usize>, marker: &str) -> Option<Range<usize>> {
    let (mut start, mut end) = (range.start, range.end);
    loop {
        let (before, after) = (&text[..start], &text[end..]);
        if before.ends_with(marker) && after.starts_with(marker) {
            return Some(start - marker.len()..end + marker.len());
        }
        let other = MARKERS
            .iter()
            .find(|m| before.ends_with(**m) && after.starts_with(**m))?;
        start -= other.len();
        end += other.len();
    }
}

fn is_url(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.contains(char::is_whitespace)
        && ["http://", "https://", "mailto:"]
            .iter()
            .any(|p| s.starts_with(p))
}

/// Makes the selection a link. A selected URL becomes the link's target,
/// with its text selected to type over; otherwise the selection is the
/// text, and the target is `clipboard` when that holds a URL, or a
/// selected `url` placeholder to type over.
pub fn insert_link(text: &str, selection: Range<usize>, clipboard: Option<&str>) -> Edit {
    let range = target(text, selection);
    let inner = &text[range.clone()];
    if is_url(inner) {
        let label = "text";
        let start = range.start + 1;
        return Edit {
            text: format!("[{label}]({inner})"),
            select: start..start + label.len(),
            range,
        };
    }
    let label = if inner.is_empty() { "text" } else { inner };
    if let Some(url) = clipboard.map(str::trim).filter(|u| is_url(u)) {
        let link = format!("[{label}]({url})");
        let select = if inner.is_empty() {
            // Nothing was selected: select the placeholder text.
            range.start + 1..range.start + 1 + label.len()
        } else {
            let end = range.start + link.len();
            end..end
        };
        return Edit {
            text: link,
            select,
            range,
        };
    }
    let url = "url";
    let start = range.start + label.len() + 3;
    Edit {
        text: format!("[{label}]({url})"),
        select: start..start + url.len(),
        range,
    }
}

/// The whole lines the selection touches. A selection that ends at the
/// start of a line leaves that line out.
fn line_range(text: &str, selection: Range<usize>) -> Range<usize> {
    let selection = clamp(text, selection);
    let start = text[..selection.start].rfind('\n').map_or(0, |i| i + 1);
    let mut last = selection.end;
    if selection.end > selection.start && last > start && text[..last].ends_with('\n') {
        last -= 1;
    }
    let end = text[last..].find('\n').map_or(text.len(), |i| last + i);
    start..end.max(start)
}

/// A line's heading level and the text after its `#`s.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &line[level..];
    if rest.is_empty() {
        return Some((level, rest));
    }
    rest.strip_prefix(' ').map(|rest| (level, rest))
}

/// A line's list marker, as (indent, marker, rest).
fn list_item(line: &str) -> Option<(&str, &str, &str)> {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = body.strip_prefix(bullet) {
            return Some((indent, bullet, rest));
        }
    }
    let digits = body.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let after = &body[digits..];
        if let Some(rest) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
        {
            return Some((indent, &body[..digits + 2], rest));
        }
    }
    None
}

fn is_numbered(marker: &str) -> bool {
    marker.starts_with(|c: char| c.is_ascii_digit())
}

fn has_style(line: &str, style: LineStyle) -> bool {
    match style {
        LineStyle::Heading(n) => heading(line).is_some_and(|(level, _)| level == n),
        LineStyle::Quote => line.starts_with('>'),
        LineStyle::Bulleted => list_item(line).is_some_and(|(_, m, _)| !is_numbered(m)),
        LineStyle::Numbered => list_item(line).is_some_and(|(_, m, _)| is_numbered(m)),
    }
}

fn remove_style(line: &str, style: LineStyle) -> String {
    match style {
        LineStyle::Heading(_) => heading(line).map_or(line, |(_, rest)| rest).to_string(),
        LineStyle::Quote => {
            let rest = &line[1..];
            rest.strip_prefix(' ').unwrap_or(rest).to_string()
        }
        LineStyle::Bulleted | LineStyle::Numbered => match list_item(line) {
            Some((indent, _, rest)) => format!("{indent}{rest}"),
            None => line.to_string(),
        },
    }
}

/// Adds `style` to a line, replacing a heading or list marker of another
/// kind. `number` counts the list's items.
fn add_style(line: &str, style: LineStyle, number: usize) -> String {
    match style {
        LineStyle::Heading(n) => {
            let rest = heading(line).map_or(line.trim_start(), |(_, rest)| rest);
            format!("{} {rest}", "#".repeat(n))
        }
        LineStyle::Quote => format!("> {line}"),
        LineStyle::Bulleted | LineStyle::Numbered => {
            let (indent, rest) = match list_item(line) {
                Some((indent, _, rest)) => (indent, rest),
                None => {
                    let body = line.trim_start();
                    (&line[..line.len() - body.len()], body)
                }
            };
            if style == LineStyle::Bulleted {
                format!("{indent}- {rest}")
            } else {
                format!("{indent}{number}. {rest}")
            }
        }
    }
}

/// Applies a line style to every line the selection touches, or takes it
/// off when they all have it already. Blank lines are left alone.
pub fn toggle_lines(text: &str, selection: Range<usize>, style: LineStyle) -> Edit {
    let range = line_range(text, selection.clone());
    let block = &text[range.clone()];
    let lines: Vec<&str> = block.split('\n').collect();
    let filled = |l: &str| !l.trim().is_empty();
    let all_styled = lines.iter().any(|l| filled(l))
        && lines
            .iter()
            .filter(|l| filled(l))
            .all(|l| has_style(l, style));
    // An empty line under the cursor still gets a heading or list marker.
    let single_empty = lines.len() == 1 && lines[0].trim().is_empty();

    let mut number = 0;
    let changed: Vec<String> = lines
        .iter()
        .map(|line| {
            if all_styled {
                return if filled(line) {
                    remove_style(line, style)
                } else {
                    line.to_string()
                };
            }
            if !filled(line) && !single_empty {
                return line.to_string();
            }
            number += 1;
            add_style(line, style, number)
        })
        .collect();
    let new = changed.join("\n");
    let end = range.start + new.len();
    // A cursor stays a cursor, at the end of its line; a selection covers
    // the changed lines.
    let select = if selection.is_empty() {
        end..end
    } else {
        range.start..end
    };
    Edit {
        range,
        text: new,
        select,
    }
}

/// Fences the lines the selection touches as a code block, or removes the
/// fences when the lines are one already.
pub fn toggle_code_block(text: &str, selection: Range<usize>) -> Edit {
    let range = line_range(text, selection);
    let block = &text[range.clone()];
    let lines: Vec<&str> = block.split('\n').collect();
    let fenced = lines.len() >= 2
        && lines[0].trim_start().starts_with("```")
        && lines[lines.len() - 1].trim() == "```";
    // The code inside the fences is selected: take off the fences around it.
    if let Some(outer) = fences_around(text, &range) {
        let end = outer.start + block.len();
        return Edit {
            range: outer.clone(),
            text: block.to_string(),
            select: outer.start..end,
        };
    }
    if fenced {
        let inner = lines[1..lines.len() - 1].join("\n");
        let end = range.start + inner.len();
        return Edit {
            range: range.clone(),
            text: inner,
            select: range.start..end,
        };
    }
    let new = format!("```\n{block}\n```");
    let start = range.start + 4;
    Edit {
        range,
        text: new,
        select: start..start + block.len(),
    }
}

/// The lines just before and after `lines`, when both are code fences:
/// the range from the opening fence to the closing one.
fn fences_around(text: &str, lines: &Range<usize>) -> Option<Range<usize>> {
    let before = text[..lines.start].strip_suffix('\n')?;
    let open_start = before.rfind('\n').map_or(0, |i| i + 1);
    let after = text[lines.end..].strip_prefix('\n')?;
    let close_len = after.find('\n').unwrap_or(after.len());
    let opening = before[open_start..].trim_start().starts_with("```");
    let closing = after[..close_len].trim() == "```";
    (opening && closing).then(|| open_start..lines.end + 1 + close_len)
}

impl Workspace {
    /// Applies an edit computed from the text and selection, as one undo
    /// step. Only in the raw editor, and only while it has the focus, so a
    /// shortcut typed in a sidebar field doesn't change the document.
    fn format_with(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&str, Range<usize>, &App) -> Edit,
    ) {
        if self.mode != ViewMode::Raw {
            self.set_notice("Switch to Raw to format text", cx);
            return;
        }
        let editor = self.editor().clone();
        if !editor
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            return;
        }
        let text = self.text(cx);
        let selection = editor.read(cx).selected_range();
        let edit = f(&text, selection, cx);
        if text.get(edit.range.clone()) == Some(edit.text.as_str()) {
            editor.update(cx, |state, cx| state.set_selected_range(edit.select, cx));
            return;
        }
        editor.update(cx, |state, cx| {
            state.set_selected_range(edit.range, cx);
            state.replace(edit.text, window, cx);
            state.set_selected_range(edit.select, cx);
        });
    }

    fn inline(&mut self, style: Inline, window: &mut Window, cx: &mut Context<Self>) {
        self.format_with(window, cx, |text, sel, _| toggle_inline(text, sel, style));
    }

    fn lines(&mut self, style: LineStyle, window: &mut Window, cx: &mut Context<Self>) {
        self.format_with(window, cx, |text, sel, _| toggle_lines(text, sel, style));
    }

    pub(crate) fn on_bold(&mut self, _: &Bold, window: &mut Window, cx: &mut Context<Self>) {
        self.inline(Inline::Bold, window, cx)
    }

    pub(crate) fn on_italic(&mut self, _: &Italic, window: &mut Window, cx: &mut Context<Self>) {
        self.inline(Inline::Italic, window, cx)
    }

    pub(crate) fn on_strikethrough(
        &mut self,
        _: &Strikethrough,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inline(Inline::Strikethrough, window, cx)
    }

    pub(crate) fn on_inline_code(
        &mut self,
        _: &InlineCode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inline(Inline::Code, window, cx)
    }

    pub(crate) fn on_insert_link(
        &mut self,
        _: &InsertLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_with(window, cx, |text, sel, cx| {
            let clipboard = cx.read_from_clipboard().and_then(|c| c.text());
            insert_link(text, sel, clipboard.as_deref())
        });
    }

    pub(crate) fn on_heading1(
        &mut self,
        _: &Heading1,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lines(LineStyle::Heading(1), window, cx)
    }

    pub(crate) fn on_heading2(
        &mut self,
        _: &Heading2,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lines(LineStyle::Heading(2), window, cx)
    }

    pub(crate) fn on_heading3(
        &mut self,
        _: &Heading3,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lines(LineStyle::Heading(3), window, cx)
    }

    pub(crate) fn on_quote(&mut self, _: &Quote, window: &mut Window, cx: &mut Context<Self>) {
        self.lines(LineStyle::Quote, window, cx)
    }

    pub(crate) fn on_bulleted_list(
        &mut self,
        _: &BulletedList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lines(LineStyle::Bulleted, window, cx)
    }

    pub(crate) fn on_numbered_list(
        &mut self,
        _: &NumberedList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lines(LineStyle::Numbered, window, cx)
    }

    pub(crate) fn on_code_block(
        &mut self,
        _: &CodeBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_with(window, cx, toggle_code_block_with_cx);
    }
}

fn toggle_code_block_with_cx(text: &str, selection: Range<usize>, _: &App) -> Edit {
    toggle_code_block(text, selection)
}

/// A labelled action for the editor's right-click menu, or a separator
/// (`None`), or a submenu.
pub enum MenuEntry {
    Item(&'static str, Box<dyn Action>),
    Separator,
    Submenu(&'static str, Vec<MenuEntry>),
}

/// What the editor's right-click menu offers, in order. Kept apart from the
/// native menu so tests can check it without popping one up.
pub fn context_menu_entries() -> Vec<MenuEntry> {
    use gpui_kit::component::input::{Copy, Cut, Paste, SelectAll};
    use MenuEntry::*;
    vec![
        Item("Cut", Box::new(Cut)),
        Item("Copy", Box::new(Copy)),
        Item("Paste", Box::new(Paste)),
        Separator,
        Item("Bold", Box::new(Bold)),
        Item("Italic", Box::new(Italic)),
        Item("Strikethrough", Box::new(Strikethrough)),
        Item("Code", Box::new(InlineCode)),
        Item("Link…", Box::new(InsertLink)),
        Separator,
        Submenu(
            "Paragraph",
            vec![
                Item("Heading 1", Box::new(Heading1)),
                Item("Heading 2", Box::new(Heading2)),
                Item("Heading 3", Box::new(Heading3)),
                Separator,
                Item("Quote", Box::new(Quote)),
                Item("Bulleted List", Box::new(BulletedList)),
                Item("Numbered List", Box::new(NumberedList)),
                Item("Code Block", Box::new(CodeBlock)),
            ],
        ),
        Separator,
        Item("Select All", Box::new(SelectAll)),
    ]
}

/// The entries as a native menu.
pub fn native_menu(entries: Vec<MenuEntry>) -> gpui_kit::component::native_menu::NativeMenu {
    entries.into_iter().fold(
        gpui_kit::component::native_menu::NativeMenu::new(),
        |menu, entry| match entry {
            MenuEntry::Item(label, action) => menu.menu(label, action),
            MenuEntry::Separator => menu.separator(),
            MenuEntry::Submenu(label, entries) => menu.submenu(label, native_menu(entries)),
        },
    )
}

/// The formatting commands as menu-bar items, for the Format menu: the
/// right-click menu's styles, without the clipboard items (the Edit menu
/// has those) and with the paragraph styles inline.
pub fn menu_items() -> Vec<MenuItem> {
    let mut out = Vec::new();
    let entries = context_menu_entries();
    let styles = entries
        .into_iter()
        .skip_while(|e| !matches!(e, MenuEntry::Separator))
        .skip(1);
    for entry in styles {
        match entry {
            MenuEntry::Item(label, action) => out.push(menu_action(label, action)),
            MenuEntry::Separator => out.push(MenuItem::separator()),
            MenuEntry::Submenu(_, entries) => {
                for entry in entries {
                    out.push(match entry {
                        MenuEntry::Item(label, action) => menu_action(label, action),
                        _ => MenuItem::separator(),
                    });
                }
            }
        }
    }
    // Drop the trailing separator and Select All, which Edit has.
    out.truncate(out.len().saturating_sub(2));
    out
}

fn menu_action(label: &str, action: Box<dyn Action>) -> MenuItem {
    MenuItem::Action {
        name: label.to_string().into(),
        action,
        os_action: None,
        checked: false,
        disabled: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{insert_link, toggle_code_block, toggle_inline, toggle_lines, Edit};
    use super::{Inline, LineStyle};

    /// Applies an edit and marks the selection with [ and ].
    fn apply(text: &str, edit: Edit) -> String {
        let mut out = text.to_string();
        out.replace_range(edit.range, &edit.text);
        out.insert(edit.select.end, ']');
        out.insert(edit.select.start, '[');
        out
    }

    /// The range of `needle` in `text`.
    fn find(text: &str, needle: &str) -> std::ops::Range<usize> {
        let start = text.find(needle).expect(needle);
        start..start + needle.len()
    }

    #[test]
    fn bold_wraps_and_unwraps() {
        let text = "Make this loud.";
        let edit = toggle_inline(text, find(text, "this"), Inline::Bold);
        assert_eq!(apply(text, edit.clone()), "Make **[this]** loud.");

        // Again, on the same selection inside the markers, takes it off.
        let bolded = "Make **this** loud.";
        let edit = toggle_inline(bolded, find(bolded, "this"), Inline::Bold);
        assert_eq!(apply(bolded, edit), "Make [this] loud.");
        // As it does with the markers selected too.
        let edit = toggle_inline(bolded, find(bolded, "**this**"), Inline::Bold);
        assert_eq!(apply(bolded, edit), "Make [this] loud.");
        // Or with another style in between.
        let both = "**_this_**";
        let edit = toggle_inline(both, find(both, "this"), Inline::Bold);
        assert_eq!(apply(both, edit), "_[this]_");
    }

    #[test]
    fn surrounding_spaces_stay_outside_the_markers() {
        let text = "a word here";
        let edit = toggle_inline(text, find(text, " word "), Inline::Italic);
        assert_eq!(apply(text, edit), "a _[word]_ here");
    }

    #[test]
    fn a_cursor_in_a_word_styles_the_word() {
        let text = "don't stop";
        let edit = toggle_inline(text, 2..2, Inline::Strikethrough);
        assert_eq!(apply(text, edit), "~~[don't]~~ stop");
    }

    #[test]
    fn a_cursor_between_words_gets_empty_markers() {
        let text = "a  b";
        let edit = toggle_inline(text, 2..2, Inline::Bold);
        assert_eq!(apply(text, edit), "a **[]** b");
    }

    #[test]
    fn code_on_several_lines_is_a_code_block() {
        let text = "Before\nlet x = 1;\nlet y = 2;\nAfter";
        let edit = toggle_inline(text, find(text, "x = 1;\nlet y"), Inline::Code);
        assert_eq!(
            apply(text, edit),
            "Before\n```\n[let x = 1;\nlet y = 2;]\n```\nAfter"
        );
        let fenced = "```\nlet x = 1;\n```";
        let edit = toggle_code_block(fenced, 0..fenced.len());
        assert_eq!(apply(fenced, edit), "[let x = 1;]");
        // Or with just the code inside selected.
        let edit = toggle_code_block(fenced, 4..5);
        assert_eq!(apply(fenced, edit), "[let x = 1;]");
    }

    #[test]
    fn links_select_what_to_type_next() {
        let text = "See the docs.";
        let docs = find(text, "the docs");
        assert_eq!(
            apply(text, insert_link(text, docs.clone(), None)),
            "See [the docs]([url])."
        );
        // A URL on the clipboard fills in the target.
        assert_eq!(
            apply(
                text,
                insert_link(text, docs, Some(" https://example.com/x \n"))
            ),
            "See [the docs](https://example.com/x)[]."
        );
        // Clipboard text that isn't a URL is ignored.
        assert_eq!(
            apply(text, insert_link(text, 13..13, Some("just words"))),
            "See the docs.[text]([url])"
        );
        // A selected URL is the target.
        let url = "Read https://example.com now";
        assert_eq!(
            apply(
                url,
                insert_link(url, find(url, "https://example.com"), None)
            ),
            "Read [[text]](https://example.com) now"
        );
    }

    #[test]
    fn headings_replace_each_other_and_toggle_off() {
        let text = "Title\n\nBody";
        let edit = toggle_lines(text, 2..2, LineStyle::Heading(2));
        assert_eq!(apply(text, edit), "## Title[]\n\nBody");
        let h2 = "## Title\n\nBody";
        let edit = toggle_lines(h2, 3..3, LineStyle::Heading(1));
        assert_eq!(apply(h2, edit), "# Title[]\n\nBody");
        let edit = toggle_lines(h2, 3..3, LineStyle::Heading(2));
        assert_eq!(apply(h2, edit), "Title[]\n\nBody");
    }

    #[test]
    fn lists_number_every_line_and_skip_blank_ones() {
        let text = "one\ntwo\n\nthree\nafter";
        let sel = find(text, "ne\ntwo\n\nthr");
        let edit = toggle_lines(text, sel.clone(), LineStyle::Numbered);
        assert_eq!(apply(text, edit), "[1. one\n2. two\n\n3. three]\nafter");

        // Bullets replace numbers, then come off again.
        let numbered = "1. one\n2. two";
        let edit = toggle_lines(numbered, 0..numbered.len(), LineStyle::Bulleted);
        assert_eq!(apply(numbered, edit), "[- one\n- two]");
        let bulleted = "- one\n  - two";
        let edit = toggle_lines(bulleted, 0..bulleted.len(), LineStyle::Bulleted);
        assert_eq!(apply(bulleted, edit), "[one\n  two]");
    }

    #[test]
    fn a_selection_ending_at_a_line_start_leaves_that_line_out() {
        let text = "quote me\nnot me";
        let edit = toggle_lines(text, 0..9, LineStyle::Quote);
        assert_eq!(apply(text, edit), "[> quote me]\nnot me");
        let quoted = "> quote me\n>\n> more";
        let edit = toggle_lines(quoted, 0..quoted.len(), LineStyle::Quote);
        assert_eq!(apply(quoted, edit), "[quote me\n\nmore]");
    }

    #[test]
    fn an_empty_line_still_gets_a_marker() {
        let text = "a\n\nb";
        let edit = toggle_lines(text, 2..2, LineStyle::Bulleted);
        assert_eq!(apply(text, edit), "a\n- []\nb");
    }
}
