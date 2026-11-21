//! Headless UI tests: the real `Workspace` in a gpui test window, driven by
//! clicks, typing and shortcuts, with a scripted assistant and a fake Jev in
//! place of the service and TypeSafe. Nothing here needs a Claude login, a
//! TypeSafe key, the Keychain or a settings file.
//!
//! Test modules import Kit types by name rather than `gpui_kit::*`, whose
//! glob would shadow Rust's `#[test]` with gpui's.

mod assistant;
mod continuous;
mod findings;
mod view;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::mpsc;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    px, size, AnyWindowHandle, App, AppContext as _, Bounds, Entity, Point, TestAppContext, Window,
    WindowBounds, WindowOptions,
};
use serde_json::{json, Value};
use wingman_client::{AuthMode, Event, ServiceInfo, TaskKind};

use crate::assistant::{Assistant, Backend, RunningTask};
use crate::jev::{Grader, Grades, JevError};
use crate::settings::Settings;
use crate::workspace::{Services, Workspace};

/// Undo and redo, as the editor binds them on this platform.
pub const UNDO: &str = if cfg!(target_os = "macos") {
    "cmd-z"
} else {
    "ctrl-z"
};
pub const REDO: &str = if cfg!(target_os = "macos") {
    "cmd-shift-z"
} else {
    "ctrl-y"
};

/// An app shortcut: Cmd on macOS, Ctrl elsewhere, as `main.rs` binds them.
pub fn shortcut(rest: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{rest}")
    } else {
        format!("ctrl-{rest}")
    }
}

/// A request the workspace sent to the assistant.
pub struct Request {
    pub kind: TaskKind,
    pub text: String,
    pub context: Option<String>,
    events: mpsc::UnboundedSender<Event>,
    pub cancelled: bool,
}

type Responder = Box<dyn Fn(TaskKind, &str) -> Vec<Event> + Send>;

#[derive(Default)]
struct FakeState {
    requests: Vec<Request>,
    responder: Option<Responder>,
}

/// A scripted assistant. Each request waits for the test to answer it,
/// unless a responder is set, which answers every request straight away.
#[derive(Clone, Default)]
pub struct FakeAssistant {
    state: Arc<Mutex<FakeState>>,
}

impl FakeAssistant {
    pub fn answer_with(&self, f: impl Fn(TaskKind, &str) -> Vec<Event> + Send + 'static) {
        self.state.lock().unwrap().responder = Some(Box::new(f));
    }

    /// What was asked so far, in order.
    pub fn requests(&self) -> Vec<(TaskKind, String)> {
        let state = self.state.lock().unwrap();
        state
            .requests
            .iter()
            .map(|r| (r.kind, r.text.clone()))
            .collect()
    }

    pub fn request_count(&self) -> usize {
        self.state.lock().unwrap().requests.len()
    }

    pub fn context(&self, index: usize) -> Option<String> {
        self.state.lock().unwrap().requests[index].context.clone()
    }

    pub fn was_cancelled(&self, index: usize) -> bool {
        self.state.lock().unwrap().requests[index].cancelled
    }

    /// Sends events for a request, as the service would.
    pub fn send(&self, index: usize, events: impl IntoIterator<Item = Event>) {
        let state = self.state.lock().unwrap();
        for event in events {
            let _ = state.requests[index].events.unbounded_send(event);
        }
    }

    fn backend(&self) -> Box<dyn Backend> {
        Box::new(self.clone())
    }
}

impl Backend for FakeAssistant {
    fn run(
        &self,
        kind: TaskKind,
        text: &str,
        context: Option<&str>,
    ) -> Result<RunningTask, String> {
        let (tx, rx) = mpsc::unbounded();
        let mut state = self.state.lock().unwrap();
        if let Some(responder) = &state.responder {
            for event in responder(kind, text) {
                let _ = tx.unbounded_send(event);
            }
        }
        let index = state.requests.len();
        state.requests.push(Request {
            kind,
            text: text.to_string(),
            context: context.map(str::to_string),
            events: tx,
            cancelled: false,
        });
        let shared = self.state.clone();
        Ok(RunningTask {
            events: rx,
            // Like the service: a cancelled task ends with "cancelled".
            cancel: Box::new(move || {
                let mut state = shared.lock().unwrap();
                let request = &mut state.requests[index];
                request.cancelled = true;
                let _ = request.events.unbounded_send(Event::Error {
                    message: "cancelled".into(),
                });
            }),
        })
    }
}

type GradeFn = Box<dyn Fn(&str) -> Result<Grades, JevError> + Send + Sync>;

/// A stand-in for Jev and the Keychain.
pub struct FakeJev {
    key: Mutex<Option<String>>,
    grade: Mutex<GradeFn>,
    /// Grades to hand out before falling back to `grade`, such as a busy reply.
    queued: Mutex<VecDeque<Result<Grades, JevError>>>,
    pub calls: AtomicUsize,
}

impl FakeJev {
    pub fn new(key: Option<&str>) -> Arc<Self> {
        Arc::new(FakeJev {
            key: Mutex::new(key.map(str::to_string)),
            grade: Mutex::new(Box::new(|_| Ok(grades(0., 0., 0.)))),
            queued: Mutex::default(),
            calls: AtomicUsize::new(0),
        })
    }

    pub fn grade_with(&self, f: impl Fn(&str) -> Result<Grades, JevError> + Send + Sync + 'static) {
        *self.grade.lock().unwrap() = Box::new(f);
    }

    pub fn queue(&self, result: Result<Grades, JevError>) {
        self.queued.lock().unwrap().push_back(result);
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn stored_key(&self) -> Option<String> {
        self.key.lock().unwrap().clone()
    }
}

impl Grader for FakeJev {
    fn load_key(&self) -> Result<Option<String>, String> {
        Ok(self.key.lock().unwrap().clone())
    }
    fn store_key(&self, key: &str) -> Result<(), String> {
        *self.key.lock().unwrap() = Some(key.to_string());
        Ok(())
    }
    fn clear_key(&self) -> Result<(), String> {
        *self.key.lock().unwrap() = None;
        Ok(())
    }
    fn grade(&self, _key: &str, _model: &str, paragraph: &str) -> Result<Grades, JevError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(result) = self.queued.lock().unwrap().pop_front() {
            return result;
        }
        (self.grade.lock().unwrap())(paragraph)
    }
}

pub fn grades(copy_edit: f32, fact_check: f32, references: f32) -> Grades {
    Grades {
        copy_edit,
        fact_check,
        references,
    }
}

/// The workspace under test, in its window, with its fakes.
pub struct Harness {
    pub window: AnyWindowHandle,
    pub workspace: Entity<Workspace>,
    pub assistant: FakeAssistant,
    pub jev: Arc<FakeJev>,
}

pub struct Setup {
    pub text: &'static str,
    pub settings: Settings,
    /// Whether the assistant starts, or fails to.
    pub assistant_fails: Option<&'static str>,
    pub jev_key: Option<&'static str>,
    pub width: f32,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            text: "",
            settings: Settings::default(),
            assistant_fails: None,
            jev_key: Some("test-key"),
            width: 1600.,
        }
    }
}

pub fn open(cx: &mut TestAppContext, setup: Setup) -> Harness {
    let assistant = FakeAssistant::default();
    let jev = FakeJev::new(setup.jev_key);
    let backend = assistant.backend();
    let failure = setup.assistant_fails;
    let services = Services {
        start_assistant: Box::new(move || match failure {
            Some(message) => Err(message.to_string()),
            None => Ok(Assistant::new(
                backend,
                ServiceInfo {
                    version: "test".into(),
                    auth: AuthMode::Subscription,
                },
            )),
        }),
        grader: jev.clone(),
        settings: Some(setup.settings),
        persist_settings: false,
    };
    let text = setup.text;
    let width = setup.width;
    let (window, workspace) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::bind_keys(cx);
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(width), px(1000.)),
        };
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            move |window, cx| {
                cx.new(|cx| {
                    let mut ws = Workspace::with_services(None, services, window, cx);
                    if !text.is_empty() {
                        ws.set_text_for_test(text, window, cx);
                    }
                    ws
                })
            },
        )
        .expect("open the test window")
    });
    let harness = Harness {
        window,
        workspace,
        assistant,
        jev,
    };
    // Let the assistant "start".
    cx.run_until_parked();
    harness.frame(cx);
    harness
}

impl Harness {
    /// Runs `f` with the window, after a fresh frame.
    pub fn with_window<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .expect("the test window is open")
    }

    pub fn frame(&self, cx: &mut TestAppContext) {
        self.with_window(cx, |_, _| ());
    }

    pub fn click(&self, cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        self.with_window(cx, |window, cx| window.click(id, cx));
        cx.run_until_parked();
    }

    pub fn press(&self, cx: &mut TestAppContext, key: &str) {
        self.with_window(cx, |window, cx| window.press(key, cx));
        cx.run_until_parked();
    }

    pub fn type_text(&self, cx: &mut TestAppContext, text: &str) {
        self.with_window(cx, |window, cx| window.input(text, cx));
        cx.run_until_parked();
    }

    pub fn exists(&self, cx: &mut TestAppContext, id: impl Into<gpui_kit::ElementId>) -> bool {
        let id = id.into();
        self.with_window(cx, |window, _| {
            window.try_find(id).is_some_and(|e| e.visible())
        })
    }

    pub fn read<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&Workspace, &App) -> R) -> R {
        cx.update(|cx| f(self.workspace.read(cx), cx))
    }

    pub fn update<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Workspace, &mut Window, &mut gpui_kit::Context<Workspace>) -> R,
    ) -> R {
        let workspace = self.workspace.clone();
        let result = cx
            .update_window(self.window, |_, window, cx| {
                workspace.update(cx, |ws, cx| f(ws, window, cx))
            })
            .expect("the test window is open");
        cx.run_until_parked();
        result
    }

    pub fn text(&self, cx: &mut TestAppContext) -> String {
        self.read(cx, |ws, cx| ws.text(cx))
    }

    pub fn notice(&self, cx: &mut TestAppContext) -> String {
        self.read(cx, |ws, _| {
            ws.notice
                .as_ref()
                .map(|n| n.to_string())
                .unwrap_or_default()
        })
    }

    /// Puts the cursor (or a selection) at the first match of `needle`.
    pub fn select(&self, cx: &mut TestAppContext, needle: &str) {
        let text = self.text(cx);
        let start = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in {text:?}"));
        self.select_range(cx, start..start + needle.len());
    }

    pub fn cursor_at(&self, cx: &mut TestAppContext, needle: &str) {
        let text = self.text(cx);
        let at = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in {text:?}"));
        self.select_range(cx, at..at);
    }

    pub fn cursor_at_end(&self, cx: &mut TestAppContext) {
        let end = self.text(cx).len();
        self.select_range(cx, end..end);
    }

    pub fn select_range(&self, cx: &mut TestAppContext, range: std::ops::Range<usize>) {
        self.update(cx, |ws, window, cx| {
            ws.editor().update(cx, |state, cx| {
                state.set_selected_range(range, cx);
                state.focus(window, cx);
            })
        });
    }

    /// The text of every visible finding, by kind, in list order.
    pub fn open_findings(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.read(cx, |ws, _| {
            ws.findings
                .iter()
                .filter(|f| f.visible())
                .map(|f| f.quote.clone())
                .collect()
        })
    }

    /// The finding ids, for building element ids such as `accept-run`.
    pub fn finding_id(&self, cx: &mut TestAppContext, quote: &str) -> u64 {
        self.read(cx, |ws, _| {
            ws.findings
                .iter()
                .rev()
                .find(|f| f.quote == quote)
                .map(|f| f.id)
                .unwrap_or_else(|| panic!("no finding for {quote:?}"))
        })
    }

    /// Moves the test clock forward, running whatever comes due.
    pub fn wait(&self, cx: &mut TestAppContext, ms: u64) {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(ms));
        cx.run_until_parked();
        self.frame(cx);
    }
}

/// The id of a button on a finding card: `{name}-{section}` with the finding id.
pub fn finding_button(name: &str, section: &str, id: u64) -> gpui_kit::ElementId {
    gpui_kit::ElementId::NamedInteger(format!("{name}-{section}").into(), id)
}

pub fn done(summary: &str, data: Value) -> Vec<Event> {
    vec![
        Event::Delta {
            text: summary.into(),
        },
        Event::Done {
            text: summary.into(),
            data: Some(data),
        },
    ]
}

pub fn copy_edit_reply(edits: &[(&str, &str)]) -> Vec<Event> {
    let edits: Vec<Value> = edits
        .iter()
        .map(|(o, r)| json!({"original": o, "replacement": r, "reason": "typo"}))
        .collect();
    done(
        &format!("{} suggested edits", edits.len()),
        json!({"summary": format!("{} suggested edits", edits.len()), "edits": edits}),
    )
}

pub fn fact_check_reply(claims: &[(&str, &str)]) -> Vec<Event> {
    let claims: Vec<Value> = claims
        .iter()
        .map(|(quote, verdict)| {
            json!({
                "quote": quote,
                "verdict": verdict,
                "explanation": format!("Checked: {quote}"),
                "sources": [{"title": "A source", "url": "https://example.com"}],
            })
        })
        .collect();
    done(
        "Checked the claims",
        json!({"summary": "Checked the claims", "claims": claims}),
    )
}

pub fn references_reply(quote: &str) -> Vec<Event> {
    done(
        "One source",
        json!({"summary": "One source", "references": [{
            "quote": quote,
            "title": "An Example Study",
            "authors": "Doe, J.",
            "year": "2020",
            "publisher": "Journal of Examples",
            "url": "https://example.com/study",
            "relation": "supports",
            "note": "Backs the claim.",
        }]}),
    )
}
