//! Quitting without losing work: ⌘Q, the window's close button and macOS
//! asking the app to terminate (Dock ▸ Quit, log out, restart) all ask to
//! save unsaved changes first, with Save, Don't Save and Cancel.
//!
//! ⌘Q and the menu item run [`request_quit`], which asks each open document
//! in turn and quits only if none of them cancels. gpui has no hook for
//! macOS's own requests to terminate, so on macOS we add
//! `applicationShouldTerminate:` to gpui's app delegate: with unsaved
//! changes it cancels the request and starts the same flow instead. With
//! nothing unsaved it lets the request through at once, so a logout isn't
//! held up.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::{AnyWindowHandle, App, Global, WeakEntity};

use crate::Quit;
use crate::workspace::Workspace;

/// Set once the user has agreed to quit, so the platform's terminate
/// request that follows goes through.
static CONFIRMED: AtomicBool = AtomicBool::new(false);
/// Whether the document has unsaved changes. The app has one window, so
/// one flag is enough; the platform hook reads it without the app.
static UNSAVED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct QuitState {
    /// Every open workspace, to ask before quitting.
    workspaces: Vec<(AnyWindowHandle, WeakEntity<Workspace>)>,
    /// A quit is waiting on the user's answers.
    asking: bool,
    /// The app asked the platform to quit, for tests.
    quit: bool,
}

impl Global for QuitState {}

/// Registers the Quit action, quits when the last window closes, and on
/// macOS catches the platform's requests to terminate.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| request_quit(cx));
    // A closed window can't be reopened, so closing it ends the app. The
    // window only closes once its own prompt said so. The About window
    // alone doesn't keep the app running.
    cx.on_window_closed(|cx, _| {
        let windows = cx.windows();
        if windows
            .iter()
            .all(|w| crate::about::is_about_window(*w, cx))
        {
            quit_now(cx);
        }
    })
    .detach();
    #[cfg(all(target_os = "macos", not(test)))]
    macos::install(cx);
}

/// Adds a workspace to the ones asked before quitting.
pub fn register(window: AnyWindowHandle, workspace: WeakEntity<Workspace>, cx: &mut App) {
    cx.default_global::<QuitState>()
        .workspaces
        .push((window, workspace));
}

/// Notes whether there's unsaved work, for the platform's terminate hook.
pub fn set_unsaved(unsaved: bool) {
    UNSAVED.store(unsaved, Ordering::SeqCst);
}

/// Asks to save each document with unsaved changes, then quits, unless the
/// user cancels.
pub fn request_quit(cx: &mut App) {
    let state = cx.default_global::<QuitState>();
    if state.asking {
        return;
    }
    log::info!("quit requested");
    state.workspaces.retain(|(_, ws)| ws.upgrade().is_some());
    let workspaces = state.workspaces.clone();
    state.asking = true;
    cx.spawn(async move |cx| {
        for (window, workspace) in workspaces {
            let answer = window.update(cx, |_, window, cx| {
                workspace.update(cx, |ws, cx| ws.confirm_discard(window, cx))
            });
            let go_on = match answer {
                Ok(Ok(answer)) => answer.await,
                // The window or workspace is gone: nothing to save there.
                _ => true,
            };
            if !go_on {
                log::info!("quit cancelled");
                cx.update(|cx| cx.default_global::<QuitState>().asking = false);
                return;
            }
        }
        cx.update(quit_now);
    })
    .detach();
}

fn quit_now(cx: &mut App) {
    log::info!("quitting");
    log::logger().flush();
    CONFIRMED.store(true, Ordering::SeqCst);
    let state = cx.default_global::<QuitState>();
    state.asking = false;
    state.quit = true;
    cx.quit();
}

/// Whether the app has asked the platform to quit.
#[cfg(test)]
pub fn quit_requested(cx: &App) -> bool {
    cx.try_global::<QuitState>().is_some_and(|s| s.quit)
}

/// Whether the platform may terminate the app without asking.
#[cfg_attr(not(all(target_os = "macos", not(test))), allow(dead_code))]
fn may_terminate() -> bool {
    CONFIRMED.load(Ordering::SeqCst) || !UNSAVED.load(Ordering::SeqCst)
}

#[cfg(all(target_os = "macos", not(test)))]
mod macos {
    use std::sync::Mutex;

    use futures::StreamExt as _;
    use futures::channel::mpsc;
    use gpui_kit::App;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{ffi, sel};

    /// NSApplicationTerminateReply.
    const TERMINATE_CANCEL: usize = 0;
    const TERMINATE_NOW: usize = 1;

    /// Wakes the app's quit flow from the delegate callback, which runs
    /// outside gpui.
    static ASK: Mutex<Option<mpsc::UnboundedSender<()>>> = Mutex::new(None);

    extern "C-unwind" fn should_terminate(
        _this: &AnyObject,
        _sel: Sel,
        _sender: *mut AnyObject,
    ) -> usize {
        if super::may_terminate() {
            return TERMINATE_NOW;
        }
        if let Some(ask) = ASK.lock().unwrap().as_ref() {
            let _ = ask.unbounded_send(());
        }
        TERMINATE_CANCEL
    }

    pub fn install(cx: &mut App) {
        let Some(class) = AnyClass::get(c"GPUIApplicationDelegate") else {
            return;
        };
        let (tx, mut rx) = mpsc::unbounded();
        *ASK.lock().unwrap() = Some(tx);
        cx.spawn(async move |cx| {
            while rx.next().await.is_some() {
                cx.update(super::request_quit);
            }
        })
        .detach();

        let callback: extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize =
            should_terminate;
        // SAFETY: gpui's delegate doesn't implement this method, the
        // function matches its signature (NSUInteger, self, _cmd, id), and
        // the type encoding says so.
        unsafe {
            let imp: Imp = std::mem::transmute(callback);
            ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                sel!(applicationShouldTerminate:),
                imp,
                c"Q@:@".as_ptr(),
            );
        }
    }
}
