//! Starts the assistant service and bridges its events into the UI thread.

use std::path::{Path, PathBuf};
use std::process::Command;

use futures::channel::mpsc;
use wingman_client::{AuthMode, Client, Event, Service, ServiceInfo, TaskKind};

/// Where the assistant service is, from the app's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    Starting,
    Ready(ServiceInfo),
    Failed(String),
}

/// A running service and a connection to it. Dropping it stops the service.
pub struct Assistant {
    // Keep the service alive for as long as the client is used.
    _service: Service,
    client: Client,
    pub info: ServiceInfo,
}

impl Assistant {
    /// Starts the service and does the handshake. Blocks for up to a few
    /// seconds, so call it off the UI thread.
    pub fn start(auth: AuthMode) -> Result<Assistant, String> {
        let exe = locate_service().ok_or_else(|| {
            "Couldn't find wingman-service. Build it with `cd service && bun run build`, \
             or set WINGMAN_SERVICE to its path."
                .to_string()
        })?;
        let service = Service::spawn(Command::new(&exe), auth)
            .map_err(|e| format!("Couldn't start {}: {e}", exe.display()))?;
        let client = service
            .connect()
            .map_err(|e| format!("Couldn't connect to the service: {e}"))?;
        let info = client.hello().map_err(|e| e.to_string())?;
        Ok(Assistant {
            _service: service,
            client,
            info,
        })
    }

    /// Starts a task and returns its events as an async stream for the UI.
    /// Returns the task id too, for cancelling.
    pub fn run(
        &self,
        task: TaskKind,
        text: &str,
        context: Option<&str>,
    ) -> Result<RunningTask, String> {
        let handle = self
            .client
            .start_task(task, text, context)
            .map_err(|e| format!("Couldn't send the request: {e}"))?;
        let (tx, rx) = mpsc::unbounded();
        let cancel_client = self.client.clone();
        let id = handle.id().to_owned();
        // The client delivers events on a blocking channel; forward them on a
        // thread so the UI can await them.
        std::thread::spawn(move || {
            for event in handle.events() {
                let last = matches!(event, Event::Done { .. } | Event::Error { .. });
                if tx.unbounded_send(event).is_err() || last {
                    break;
                }
            }
        });
        Ok(RunningTask {
            events: rx,
            cancel: Box::new(move || {
                let _ = cancel_client.cancel(&id);
            }),
        })
    }
}

pub struct RunningTask {
    pub events: mpsc::UnboundedReceiver<Event>,
    pub cancel: Box<dyn Fn() + Send>,
}

/// Finds the wingman-service executable, in this order:
/// 1. `WINGMAN_SERVICE`
/// 2. next to the app's executable
/// 3. inside a macOS app bundle (`Contents/Resources`)
/// 4. the repository's `service/dist`, for development builds
pub fn locate_service() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WINGMAN_SERVICE") {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    let mut candidates = Vec::new();
    if let Some(dir) = exe_dir {
        candidates.push(dir.join("wingman-service"));
        candidates.push(dir.join("../Resources/wingman-service"));
    }
    candidates
        .push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../service/dist/wingman-service"));
    candidates.into_iter().find(|p| p.is_file())
}

/// The sign-in mode the app starts the service with: subscription unless
/// `WINGMAN_AUTH=api_key`.
pub fn auth_mode_from_env() -> AuthMode {
    match std::env::var("WINGMAN_AUTH").as_deref() {
        Ok("api_key") | Ok("api-key") => AuthMode::ApiKey,
        _ => AuthMode::Subscription,
    }
}

impl ServiceStatus {
    pub fn label(&self) -> String {
        match self {
            ServiceStatus::Starting => "Assistant starting…".into(),
            ServiceStatus::Ready(info) => match info.auth {
                AuthMode::Subscription => "Assistant ready · Claude subscription".into(),
                AuthMode::ApiKey => "Assistant ready · API key".into(),
            },
            ServiceStatus::Failed(_) => "Assistant unavailable".into(),
        }
    }
}

pub fn task_title(task: TaskKind) -> &'static str {
    match task {
        TaskKind::CopyEdit => "Copy-edit",
        TaskKind::FactCheck => "Fact-check",
        TaskKind::FindReferences => "References",
    }
}
