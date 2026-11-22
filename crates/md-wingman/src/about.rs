//! The About window: the version, who made the app, its license, and links
//! to the project on GitHub and its website.
//!
//! It opens only from the app menu, never on its own, so it can't get in
//! the way of writing. Asking again while it's open brings the same window
//! to the front.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::*;

use crate::{About, CloseAbout};

/// The app's version, from Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The project on GitHub.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// The project website, served by GitHub Pages from `site/`.
pub const WEBSITE: &str = "https://md-wingman.dev/";
/// The license, by name and in full.
pub const LICENSE_NAME: &str = "Apache License 2.0";
pub const LICENSE_URL: &str = "https://github.com/deanberris/md-wingman/blob/main/LICENSE";

/// The repository's contributors list, which the window shows.
const CONTRIBUTORS_MD: &str = include_str!("../../../CONTRIBUTORS.md");

/// One person from `CONTRIBUTORS.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contributor {
    pub name: String,
    pub note: Option<String>,
}

/// The people listed in `CONTRIBUTORS.md`: each line that starts with `- `,
/// as `- Name — note`, the note being optional.
pub fn contributors() -> Vec<Contributor> {
    parse_contributors(CONTRIBUTORS_MD)
}

fn parse_contributors(text: &str) -> Vec<Contributor> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(|entry| match entry.split_once(" — ") {
            Some((name, note)) => Contributor {
                name: name.trim().to_string(),
                note: Some(note.trim().to_string()).filter(|n| !n.is_empty()),
            },
            None => Contributor {
                name: entry.trim().to_string(),
                note: None,
            },
        })
        .filter(|c| !c.name.is_empty())
        .collect()
}

/// The open About window, if any.
#[derive(Default)]
struct AboutWindow(Option<AnyWindowHandle>);

impl Global for AboutWindow {}

/// Registers the About action and the keys that close the window.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &About, cx| open(cx));
    cx.on_window_closed(|cx, closed| {
        let about = cx.default_global::<AboutWindow>();
        if about.0.is_some_and(|w| w.window_id() == closed) {
            about.0 = None;
        }
    })
    .detach();
    cx.bind_keys([
        KeyBinding::new("escape", CloseAbout, Some("AboutView")),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            CloseAbout,
            Some("AboutView"),
        ),
    ]);
}

/// Whether `window` is the About window.
pub fn is_about_window(window: AnyWindowHandle, cx: &App) -> bool {
    cx.try_global::<AboutWindow>()
        .and_then(|w| w.0)
        .is_some_and(|w| w == window)
}

/// The open About window, for tests.
#[cfg(test)]
pub fn window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<AboutWindow>().and_then(|w| w.0)
}

/// Opens the About window, or brings it to the front if it's open.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.default_global::<AboutWindow>().0
        && cx
            .update_window(handle, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let width = px(440.);
    let height = px(520.);
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("About md-wingman".into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(width, height),
            cx,
        ))),
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| AboutView::new(window, cx))
    }) {
        Ok((window, _)) => cx.set_global(AboutWindow(Some(window))),
        Err(e) => eprintln!("md-wingman: couldn't open the About window: {e}"),
    }
}

/// Opens a link in the browser. Tests record it instead.
fn open_link(url: &str, cx: &mut App) {
    #[cfg(test)]
    {
        cx.default_global::<OpenedLinks>().0.push(url.to_string());
    }
    #[cfg(not(test))]
    cx.open_url(url);
}

/// Links the About window opened, for tests.
#[cfg(test)]
#[derive(Default)]
pub struct OpenedLinks(pub Vec<String>);

#[cfg(test)]
impl Global for OpenedLinks {}

pub struct AboutView {
    focus_handle: FocusHandle,
    contributors: Vec<Contributor>,
}

impl AboutView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        AboutView {
            focus_handle,
            contributors: contributors(),
        }
    }

    fn on_close(&mut self, _: &CloseAbout, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }
}

impl Focusable for AboutView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AboutView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mono = theme.mono_font_family.clone();

        let heading = |text: &str| {
            div()
                .text_xs()
                .font_family(mono.clone())
                .text_color(theme.muted_foreground)
                .child(format!("── {text} "))
        };
        let link = |id: &'static str, label: String, url: &'static str| {
            div()
                .id(id)
                .test_support()
                .text_sm()
                .text_color(theme.link)
                .underline()
                .cursor_pointer()
                .child(label)
                .on_click(move |_, _, cx| open_link(url, cx))
        };

        let mut people = v_flex().gap_1();
        for (i, c) in self.contributors.iter().enumerate() {
            people = people.child(
                h_flex()
                    .id(("contributor", i))
                    .test_support()
                    .gap_2()
                    .text_sm()
                    .child(div().font_weight(FontWeight::MEDIUM).child(c.name.clone()))
                    .children(
                        c.note
                            .clone()
                            .map(|note| div().text_color(theme.muted_foreground).child(note)),
                    ),
            );
        }

        v_flex()
            .id("about")
            .key_context("AboutView")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close))
            .size_full()
            .p_8()
            .gap_5()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_2xl()
                            .font_family(mono.clone())
                            .font_weight(FontWeight::BOLD)
                            .child("md-wingman"),
                    )
                    .child(
                        div()
                            .id("about-version")
                            .test_support()
                            .text_sm()
                            .font_family(mono.clone())
                            .text_color(theme.muted_foreground)
                            .child(format!("version {VERSION}")),
                    )
                    .child(
                        div()
                            .pt_2()
                            .text_sm()
                            .child("A Markdown editor with a Claude writing assistant beside it."),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(heading("Contributors"))
                    .child(people),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(heading("License"))
                    .child(
                        div()
                            .text_sm()
                            .child(format!("Released under the {LICENSE_NAME}.")),
                    )
                    .child(link(
                        "about-license",
                        "Read the license".into(),
                        LICENSE_URL,
                    )),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(heading("Links"))
                    .child(link(
                        "about-website",
                        format!("Website · {}", display_url(WEBSITE)),
                        WEBSITE,
                    ))
                    .child(link(
                        "about-github",
                        format!("GitHub · {}", display_url(REPOSITORY)),
                        REPOSITORY,
                    )),
            )
    }
}

/// A URL without its scheme or trailing slash, for showing.
fn display_url(url: &str) -> &str {
    url.trim_start_matches("https://").trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn parses_names_and_notes() {
        let text = "# Contributors\n\nIntro text - not a person.\n\n\
                    - Ada Lovelace — notes\n- Grace Hopper\n-  \n";
        assert_eq!(
            parse_contributors(text),
            vec![
                Contributor {
                    name: "Ada Lovelace".into(),
                    note: Some("notes".into())
                },
                Contributor {
                    name: "Grace Hopper".into(),
                    note: None
                },
            ]
        );
    }

    #[::core::prelude::v1::test]
    fn the_repository_lists_its_maintainer() {
        assert!(
            contributors()
                .iter()
                .any(|c| c.name == "Dean Michael Berris")
        );
    }

    #[::core::prelude::v1::test]
    fn links_point_at_the_project() {
        assert_eq!(REPOSITORY, "https://github.com/deanberris/md-wingman");
        assert_eq!(display_url(WEBSITE), "md-wingman.dev");
    }
}
