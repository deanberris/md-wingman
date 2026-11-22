# UI toolkit: gpui or iced

**Recommendation: gpui, with gpui-component.** iced is the fallback if gpui gets in the way during #3.

This comparison is based on the crates' documentation, features and licences, as of November 2026. Neither toolkit was
run on macOS for it. The first days of #3 are the real test.

## What the editor needs

1. Good typography: proportional and monospace fonts, ligatures, proper line height, and crisp rendering on Retina screens.
2. A multi-line Markdown editor with syntax highlighting.
3. A rendered Markdown preview, for the raw/preview toggle.
4. Inline decorations on the text, such as underlines, highlights and ghost text, so that copy-edit suggestions and
   fact-check flags can sit in the document (#4).
5. A sidebar, panels and a native macOS feel.
6. Apache-2.0-compatible licensing.

## Comparison

| | gpui 0.2 (+ gpui-component 0.10) | iced 0.14 |
|---|---|---|
| Licence | Apache-2.0 (both crates) | MIT |
| Text rendering on macOS | Core Text, so native shaping and font fallback, and the same text quality as Zed | cosmic-text, rendered through wgpu: good, but not native-looking |
| Editor widget | gpui-component's `Input`: a multi-line code editor with tree-sitter highlighting (`tree-sitter-markdown` feature) | `text_editor`: multi-line, with a syntect highlighter. Simpler, and fewer hooks for decorations |
| Markdown preview | gpui-component's Markdown text view | `markdown` widget (feature flag) |
| Inline decorations | Custom elements and text runs are first-class. Zed's whole editor is built on them | Hard: you would have to extend or replace `text_editor` |
| Architecture | Entities and views with a retained element tree. Steeper to learn | Elm-style update and view. Easy to reason about |
| Maturity | Pre-1.0, with API churn. Docs are thin, so you learn from examples and Zed's source | Pre-1.0 as well, but has longer-standing public releases and more tutorials |
| Proven for | Zed, a large production editor | Many small and medium apps. COSMIC desktop apps |

## Why gpui

Typography and inline decorations matter most to this project, and gpui is stronger on both. Inline accept or reject for
copy edits (#4) is the hardest UI piece. gpui was built for exactly that kind of editor, while iced would need its
editor widget reworked.

gpui-component already provides the editor and the Markdown preview, so #3 does not start from scratch.

## Watch out for

- **Licensing.** Zed's own `editor` crate is GPL, so we must not copy code from it, only from gpui and gpui-component,
  which are both Apache-2.0.
- **API churn.** Pin exact versions and upgrade on purpose.
- **Fallback.** If the editor input can't carry decorations well, or text rendering disappoints on a real Mac, switch to
  iced early in #3, before much UI code exists. `wingman-client` doesn't depend on either toolkit.
