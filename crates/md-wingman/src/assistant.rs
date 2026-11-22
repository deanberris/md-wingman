//! Starts the assistant service and bridges its events into the UI thread.

use std::path::{Path, PathBuf};
use std::process::Command;

use futures::channel::mpsc;
use wingman_client::{AuthMode, Client, Event, Service, ServiceInfo, TaskKind};

use crate::provider::ProviderSettings;

/// Where the assistant service is, from the app's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    Starting,
    Ready(ServiceInfo),
    Failed(String),
}

/// Something that runs assistant tasks: the real service, or a fake in tests.
pub trait Backend: Send {
    /// Starts a task and returns its events as an async stream for the UI,
    /// with a way to cancel it.
    fn run(&self, task: TaskKind, text: &str, context: Option<&str>)
        -> Result<RunningTask, String>;
}

/// A connection to the assistant, and what it said about itself.
pub struct Assistant {
    backend: Box<dyn Backend>,
    pub info: ServiceInfo,
}

impl Assistant {
    pub fn new(backend: Box<dyn Backend>, info: ServiceInfo) -> Self {
        Assistant { backend, info }
    }

    /// Starts the service and does the handshake. Blocks for up to a few
    /// seconds, so call it off the UI thread.
    pub fn start(connection: Connection) -> Result<Assistant, String> {
        let exe = locate_service().ok_or_else(|| {
            "Couldn't find wingman-service. Build it with `cd service && bun run build`, \
             or set WINGMAN_SERVICE to its path."
                .to_string()
        })?;
        let mut command = Command::new(&exe);
        let auth = connection.configure(&mut command);
        let service = Service::spawn(command, auth)
            .map_err(|e| format!("Couldn't start {}: {e}", exe.display()))?;
        let client = service
            .connect()
            .map_err(|e| format!("Couldn't connect to the service: {e}"))?;
        let info = client.hello().map_err(|e| e.to_string())?;
        Ok(Assistant::new(
            Box::new(ServiceBackend {
                _service: service,
                client,
            }),
            info,
        ))
    }

    /// Starts a task and returns its events as an async stream for the UI.
    pub fn run(
        &self,
        task: TaskKind,
        text: &str,
        context: Option<&str>,
    ) -> Result<RunningTask, String> {
        self.backend.run(task, text, context)
    }
}

/// The real backend: a running service and a connection to it. Dropping it
/// stops the service.
struct ServiceBackend {
    // Keep the service alive for as long as the client is used.
    _service: Service,
    client: Client,
}

impl Backend for ServiceBackend {
    fn run(
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

/// How the service reaches a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// Claude Code as it is: the Claude plan, or whatever `WINGMAN_AUTH`
    /// and the other `WINGMAN_*` variables in the environment say.
    Claude(AuthMode),
    /// The model provider from the settings, with its token from the
    /// Keychain.
    Provider {
        settings: ProviderSettings,
        token: Option<String>,
    },
}

impl Connection {
    /// Sets the service's environment for this connection, and returns the
    /// mode to start it in.
    pub fn configure(&self, command: &mut Command) -> AuthMode {
        match self {
            Connection::Claude(auth) => *auth,
            Connection::Provider { settings, token } => {
                command
                    .env("WINGMAN_PROVIDER_NAME", settings.display_name())
                    .env("WINGMAN_PROVIDER_URL", settings.base_url.trim())
                    .env("WINGMAN_MODEL", settings.model.trim())
                    .env(
                        "WINGMAN_WEB_SEARCH",
                        if settings.web_search { "1" } else { "0" },
                    )
                    .env_remove("WINGMAN_PROVIDER_TOKEN")
                    .env_remove("WINGMAN_FAST_MODEL");
                if !settings.fast_model.trim().is_empty() {
                    command.env("WINGMAN_FAST_MODEL", settings.fast_model.trim());
                }
                if let Some(token) = token.as_deref().filter(|t| !t.is_empty()) {
                    command.env("WINGMAN_PROVIDER_TOKEN", token);
                }
                AuthMode::Provider
            }
        }
    }
}

/// The sign-in mode `WINGMAN_AUTH` asks for, if it's set. It wins over the
/// settings, for development and for anyone who'd rather configure it in
/// the environment.
pub fn auth_mode_from_env() -> Option<AuthMode> {
    match std::env::var("WINGMAN_AUTH").ok()?.trim() {
        "api_key" | "api-key" => Some(AuthMode::ApiKey),
        "provider" => Some(AuthMode::Provider),
        "subscription" => Some(AuthMode::Subscription),
        _ => None,
    }
}

impl ServiceStatus {
    pub fn label(&self) -> String {
        match self {
            ServiceStatus::Starting => "Assistant starting…".into(),
            ServiceStatus::Ready(info) => match info.auth {
                AuthMode::Subscription => "Assistant ready · Claude subscription".into(),
                AuthMode::ApiKey => "Assistant ready · API key".into(),
                AuthMode::Provider => format!(
                    "Assistant ready · {} · {}",
                    info.provider.as_deref().unwrap_or("model provider"),
                    info.model.as_deref().unwrap_or("default model")
                ),
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

#[cfg(test)]
mod tests {
    use super::Connection;
    use crate::provider::ProviderSettings;
    use std::process::Command;
    use wingman_client::AuthMode;

    fn env_of(command: &Command, name: &str) -> Option<Option<String>> {
        command
            .get_envs()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    #[test]
    fn a_provider_is_passed_to_the_service_in_its_environment() {
        let mut command = Command::new("wingman-service");
        let connection = Connection::Provider {
            settings: ProviderSettings {
                enabled: true,
                name: String::new(),
                base_url: "http://localhost:11434".into(),
                model: "qwen3".into(),
                fast_model: String::new(),
                web_search: false,
            },
            token: None,
        };
        assert_eq!(connection.configure(&mut command), AuthMode::Provider);
        let env = |name| env_of(&command, name);
        assert_eq!(
            env("WINGMAN_PROVIDER_NAME"),
            Some(Some("localhost:11434".into()))
        );
        assert_eq!(
            env("WINGMAN_PROVIDER_URL"),
            Some(Some("http://localhost:11434".into()))
        );
        assert_eq!(env("WINGMAN_MODEL"), Some(Some("qwen3".into())));
        assert_eq!(env("WINGMAN_WEB_SEARCH"), Some(Some("0".into())));
        // No token: none is passed, not even one from the app's environment.
        assert_eq!(env("WINGMAN_PROVIDER_TOKEN"), Some(None));
        assert_eq!(env("WINGMAN_FAST_MODEL"), Some(None));

        let mut command = Command::new("wingman-service");
        let Connection::Provider { settings, .. } = connection else {
            unreachable!()
        };
        let connection = Connection::Provider {
            settings: ProviderSettings {
                fast_model: "qwen3:4b".into(),
                ..settings
            },
            token: Some("tok".into()),
        };
        connection.configure(&mut command);
        assert_eq!(
            env_of(&command, "WINGMAN_PROVIDER_TOKEN"),
            Some(Some("tok".into()))
        );
        assert_eq!(
            env_of(&command, "WINGMAN_FAST_MODEL"),
            Some(Some("qwen3:4b".into()))
        );
    }

    #[test]
    fn claude_leaves_the_environment_alone() {
        let mut command = Command::new("wingman-service");
        assert_eq!(
            Connection::Claude(AuthMode::ApiKey).configure(&mut command),
            AuthMode::ApiKey
        );
        assert_eq!(command.get_envs().count(), 0);
    }
}
