//! md-wingman: a Markdown editor with a Claude writing assistant beside it.
//!
//! Usage: md-wingman [FILE]

mod assistant;
mod document;
mod findings;
mod reflow;
mod ruler;
mod runs;
mod settings;
mod store;
mod suggestions;
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
        CopyEdit,
        FactCheck,
        FindReferences,
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

fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);

            cx.bind_keys([
                KeyBinding::new(key!("n"), NewFile, None),
                KeyBinding::new(key!("o"), Open, None),
                KeyBinding::new(key!("s"), Save, None),
                KeyBinding::new(key!("shift-s"), SaveAs, None),
                KeyBinding::new(key!("p"), TogglePreview, None),
                KeyBinding::new(key!("e"), CopyEdit, None),
                KeyBinding::new(key!("shift-c"), FactCheck, None),
                KeyBinding::new(key!("shift-r"), FindReferences, None),
                KeyBinding::new(key!("alt-q"), Reflow, None),
                KeyBinding::new(key!("q"), Quit, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus([
                Menu::new("md-wingman").items([MenuItem::action("Quit md-wingman", Quit)]),
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
                Menu::new("View").items([
                    MenuItem::action("Toggle Preview", TogglePreview),
                    MenuItem::action("Toggle Ruler", ToggleRuler),
                ]),
                Menu::new("Assistant").items([
                    MenuItem::action("Copy-edit", CopyEdit),
                    MenuItem::action("Fact-check", FactCheck),
                    MenuItem::action("Find References", FindReferences),
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
            cx.activate(true);
        });
}
