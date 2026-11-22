//! md-wingman: a Markdown editor with a Claude writing assistant beside it.
//!
//! Usage: md-wingman [FILE]

mod about;
mod appearance;
mod assistant;
mod continuous;
mod dictation;
mod document;
mod findings;
mod format;
mod guide;
mod jev;
mod logging;
mod prefilter;
mod provider;
mod quit;
mod reflow;
mod review;
mod ruler;
mod runs;
mod settings;
mod setup;
mod store;
mod suggestions;
#[cfg(test)]
mod ui_tests;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::*;

use crate::workspace::Workspace;

actions!(
    md_wingman,
    [
        NewFile,
        Open,
        Save,
        SaveAs,
        TogglePreview,
        ToggleRuler,
        Reflow,
        Bold,
        Italic,
        Strikethrough,
        InlineCode,
        InsertLink,
        Heading1,
        Heading2,
        Heading3,
        Quote,
        BulletedList,
        NumberedList,
        CodeBlock,
        CopyEdit,
        FactCheck,
        FindReferences,
        ReviewDocument,
        ToggleContinuous,
        ToggleJevPrefilter,
        SetJevKey,
        ModelProvider,
        ShowSetup,
        ToggleDarkMode,
        AppearanceAndFonts,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        ShowLogFile,
        ShowCrashReports,
        ReportProblem,
        ToggleVerboseLogging,
        MarkdownGuide,
        About,
        CloseAbout,
        Quit
    ]
);

/// A shortcut with Cmd on macOS and Ctrl elsewhere.
macro_rules! key {
    ($rest:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $rest)
        } else {
            concat!("ctrl-", $rest)
        }
    };
}

/// The app's keyboard shortcuts.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(key!("n"), NewFile, None),
        KeyBinding::new(key!("o"), Open, None),
        KeyBinding::new(key!("s"), Save, None),
        KeyBinding::new(key!("shift-s"), SaveAs, None),
        KeyBinding::new(key!("p"), TogglePreview, None),
        KeyBinding::new(key!("e"), CopyEdit, None),
        KeyBinding::new(key!("shift-c"), FactCheck, None),
        KeyBinding::new(key!("shift-r"), FindReferences, None),
        KeyBinding::new(key!("shift-d"), ReviewDocument, None),
        KeyBinding::new(key!("alt-q"), Reflow, None),
        KeyBinding::new(key!("b"), Bold, None),
        KeyBinding::new(key!("i"), Italic, None),
        KeyBinding::new(key!("k"), InsertLink, None),
        KeyBinding::new(key!("shift-x"), Strikethrough, None),
        KeyBinding::new(key!("shift-k"), ToggleContinuous, None),
        KeyBinding::new(key!("="), IncreaseFontSize, None),
        KeyBinding::new(key!("-"), DecreaseFontSize, None),
        KeyBinding::new(key!("0"), ResetFontSize, None),
        KeyBinding::new(key!("/"), MarkdownGuide, None),
        KeyBinding::new(key!("q"), Quit, None),
    ]);
}

fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    logging::init();
    if let Some(p) = &path {
        log::info!("opening {} from the command line", p.display());
    }

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);

            bind_keys(cx);
            quit::init(cx);
            about::init(cx);
            cx.set_menus([
                Menu::new("md-wingman").items([
                    MenuItem::action("About md-wingman", About),
                    MenuItem::action("Set Up md-wingman…", ShowSetup),
                    MenuItem::separator(),
                    MenuItem::action("Quit md-wingman", Quit),
                ]),
                Menu::new("File").items([
                    MenuItem::action("New", NewFile),
                    MenuItem::action("Open…", Open),
                    MenuItem::separator(),
                    MenuItem::action("Save", Save),
                    MenuItem::action("Save As…", SaveAs),
                ]),
                Menu::new("Edit").items([
                    MenuItem::action("Undo", Undo),
                    MenuItem::action("Redo", Redo),
                    MenuItem::separator(),
                    MenuItem::action("Cut", Cut),
                    MenuItem::action("Copy", Copy),
                    MenuItem::action("Paste", Paste),
                    MenuItem::action("Select All", SelectAll),
                    MenuItem::separator(),
                    MenuItem::action("Reflow Paragraph", Reflow),
                ]),
                Menu::new("Format").items(format::menu_items()),
                Menu::new("View").items([
                    MenuItem::action("Toggle Preview", TogglePreview),
                    MenuItem::action("Toggle Ruler", ToggleRuler),
                    MenuItem::separator(),
                    MenuItem::action("Toggle Dark Mode", ToggleDarkMode),
                    MenuItem::action("Appearance and Fonts…", AppearanceAndFonts),
                    MenuItem::action("Bigger Text", IncreaseFontSize),
                    MenuItem::action("Smaller Text", DecreaseFontSize),
                    MenuItem::action("Actual Size", ResetFontSize),
                ]),
                Menu::new("Assistant").items([
                    MenuItem::action("Copy-edit", CopyEdit),
                    MenuItem::action("Fact-check", FactCheck),
                    MenuItem::action("Find References", FindReferences),
                    MenuItem::action("Review Whole Document", ReviewDocument),
                    MenuItem::separator(),
                    MenuItem::action("Check As You Write", ToggleContinuous),
                    MenuItem::action("Jev Pre-filter", ToggleJevPrefilter),
                    MenuItem::action("Set TypeSafe API Key…", SetJevKey),
                    MenuItem::separator(),
                    MenuItem::action("Model Provider…", ModelProvider),
                    MenuItem::action("Setup…", ShowSetup),
                ]),
                Menu::new("Help").items([
                    MenuItem::action("Markdown Guide", MarkdownGuide),
                    MenuItem::separator(),
                    MenuItem::action("Report a Problem…", ReportProblem),
                    MenuItem::separator(),
                    MenuItem::action("Show Log File", ShowLogFile),
                    MenuItem::action("Show Crash Reports", ShowCrashReports),
                    MenuItem::action("Verbose Logging", ToggleVerboseLogging),
                ]),
            ]);

            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("md-wingman".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1680.), px(1000.)),
                    cx,
                ))),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Workspace::new(path.clone(), window, cx))
            })
            .expect("couldn't open the main window");
            log::info!("main window open");
            cx.activate(true);
        });
}
