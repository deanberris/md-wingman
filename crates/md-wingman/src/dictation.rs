//! Smoothing over text that dictation apps paste in.
//!
//! Wispr Flow and similar apps insert each phrase by pasting it, and add a
//! space in front when they think the cursor sits after a word. They can't
//! read the editor's text, so they guess, and a phrase dictated at the start
//! of a new paragraph arrives as " Another sentence.", leaving a stray space
//! at the start of the line.

use gpui_kit::component::input::Paste;
use gpui_kit::*;

use crate::workspace::{ViewMode, Workspace};

/// The paste to insert instead of `pasted`, when it would put a single
/// stray space at the start of a line. `before` is the document up to where
/// the paste goes.
pub fn without_stray_space<'a>(before: &str, pasted: &'a str) -> Option<&'a str> {
    let at_line_start = before.is_empty() || before.ends_with('\n');
    let rest = pasted.strip_prefix(' ')?;
    let next = rest.chars().next()?;
    // Leave deliberate indentation alone: two or more spaces, or a tab, is
    // someone indenting, not a dictation app's guess.
    (at_line_start && !next.is_whitespace()).then_some(rest)
}

impl Workspace {
    /// Runs before the editor's own paste, and takes it over only to drop a
    /// stray leading space, as one undoable change.
    pub(crate) fn on_paste_capture(
        &mut self,
        _: &Paste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode != ViewMode::Raw {
            return;
        }
        let Some(pasted) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        let text = self.text(cx);
        let selection = self.editor().read(cx).selected_range();
        let Some(before) = text.get(..selection.start) else {
            return;
        };
        let Some(trimmed) = without_stray_space(before, &pasted) else {
            return;
        };
        let trimmed = trimmed.to_string();
        self.editor().update(cx, |state, cx| {
            state.replace(trimmed, window, cx);
        });
        cx.stop_propagation();
    }
}

#[cfg(test)]
mod tests {
    use super::without_stray_space;

    #[test]
    fn drops_one_space_at_the_start_of_a_line() {
        assert_eq!(
            without_stray_space("Sentence.\n\n", " Another sentence."),
            Some("Another sentence.")
        );
        assert_eq!(without_stray_space("", " First."), Some("First."));
    }

    #[test]
    fn keeps_spaces_mid_line_and_deliberate_indents() {
        assert_eq!(without_stray_space("Sentence.", " Another."), None);
        assert_eq!(without_stray_space("Sentence.\n", "    code"), None);
        assert_eq!(without_stray_space("Sentence.\n", "  two"), None);
        assert_eq!(without_stray_space("Sentence.\n", " \tx"), None);
        assert_eq!(without_stray_space("Sentence.\n", " "), None);
        assert_eq!(without_stray_space("Sentence.\n", "No space."), None);
    }
}
