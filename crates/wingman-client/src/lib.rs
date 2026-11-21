//! Client for the md-wingman assistant service.
//!
//! The service is a separate process (see `service/` in the repository) that
//! talks to Claude through the Claude Agent SDK. This crate starts it, connects
//! to it over a Unix socket, and streams replies back. It uses plain threads and
//! channels so it fits any UI toolkit's event loop.
//!
//! It logs through the `log` crate, under the `wingman_client` target. The
//! service's own log lines (its stderr) are forwarded under the `service`
//! target at the level the service gave them, so whatever logger the app
//! sets up gets one story from both processes.
//!
//! ```no_run
//! use std::process::Command;
//! use wingman_client::{AuthMode, Event, Service, TaskKind};
//!
//! let service = Service::spawn(Command::new("wingman-service"), AuthMode::Subscription)?;
//! let client = service.connect()?;
//! let task = client.start_task(TaskKind::CopyEdit, "Their going home.", None)?;
//! for event in task.events() {
//!     match event {
//!         Event::Delta { text } => print!("{text}"),
//!         Event::Done { .. } | Event::Error { .. } => break,
//!         _ => {}
//!     }
//! }
//! # Ok::<(), std::io::Error>(())
//! ```

mod protocol;

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub use protocol::{AuthMode, Event, TaskKind, PROTOCOL_VERSION};
use protocol::{Envelope, Request};

/// What the service reports about itself in the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    pub version: String,
    /// The credential the service is using, so the UI can show it.
    pub auth: AuthMode,
}

/// How long [`Service::spawn`] waits for the service to start listening.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// How many of the service's last log lines are kept, to explain a failed
/// start.
const KEPT_LINES: usize = 20;

/// A running service process. Dropping it stops the process.
pub struct Service {
    child: Child,
    socket_path: PathBuf,
    /// The service's last few stderr lines.
    recent: Arc<Mutex<Vec<String>>>,
}

impl Service {
    /// Starts the service from `command` (typically the `wingman-service`
    /// executable, with any other environment variables already set) and waits
    /// until it accepts connections. `auth` picks which credential it uses;
    /// [`AuthMode::Subscription`] is the default.
    ///
    /// The service's stdin is held open by this process. If the app exits
    /// without dropping the `Service`, stdin closes and the service exits too.
    pub fn spawn(mut command: Command, auth: AuthMode) -> io::Result<Service> {
        let socket_path = default_socket_path();
        let started = Instant::now();
        log::info!(
            "starting service {:?} ({} sign-in, socket {})",
            command.get_program(),
            auth.as_env_value(),
            socket_path.display()
        );
        // The service logs at the app's level, unless told otherwise.
        if std::env::var_os("WINGMAN_LOG").is_none() {
            command.env("WINGMAN_LOG", log::max_level().as_str().to_lowercase());
        }
        let mut child = command
            .env("WINGMAN_AUTH", auth.as_env_value())
            .arg("--socket")
            .arg(&socket_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .inspect_err(|e| log::error!("couldn't start the service: {e}"))?;
        let recent = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let recent = recent.clone();
            thread::Builder::new()
                .name("wingman-service-log".into())
                .spawn(move || forward_log(stderr, &recent))?;
        }
        log::debug!("service pid {}", child.id());
        let mut service = Service {
            child,
            socket_path,
            recent,
        };
        match service.wait_until_listening() {
            Ok(()) => {
                log::info!("service listening after {:?}", started.elapsed());
                Ok(service)
            }
            Err(e) => {
                // Give the log thread a moment to catch the service's last words.
                thread::sleep(Duration::from_millis(100));
                let last = service.recent_lines();
                log::error!("service failed to start: {e}");
                Err(match last.last() {
                    Some(line) => io::Error::new(e.kind(), format!("{e}: {line}")),
                    None => e,
                })
            }
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// The service's last few log lines, oldest first.
    pub fn recent_lines(&self) -> Vec<String> {
        self.recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Opens a new connection. One connection can carry many requests.
    pub fn connect(&self) -> io::Result<Client> {
        Client::connect(&self.socket_path)
    }

    fn wait_until_listening(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!(
                    "service exited during startup ({status})"
                )));
            }
            if UnixStream::connect(&self.socket_path).is_ok() {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "service did not start listening",
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // Closing stdin asks the service to exit cleanly; kill it if it hasn't.
        log::info!("stopping the service");
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    log::info!("service exited ({status})");
                    return;
                }
                Err(e) => log::warn!("couldn't check on the service: {e}"),
                Ok(None) => {}
            }
            thread::sleep(Duration::from_millis(20));
        }
        log::warn!("service didn't exit within 2s; killing it");
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Forwards the service's stderr into the log, line by line, keeping the
/// last few for error messages.
fn forward_log(stderr: impl io::Read, recent: &Mutex<Vec<String>>) {
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let (level, message) = service_line(&line);
        log::log!(target: "service", level, "{message}");
        let mut recent = recent.lock().unwrap_or_else(|e| e.into_inner());
        if recent.len() == KEPT_LINES {
            recent.remove(0);
        }
        recent.push(message.to_string());
    }
    log::debug!("service closed its log");
}

/// Splits a service log line, `"INFO message"`, into its level and message.
/// Lines without a level (such as a crash's output) are warnings.
pub fn service_line(line: &str) -> (log::Level, &str) {
    let (first, rest) = line.split_once(' ').unwrap_or((line, ""));
    let level = match first {
        "ERROR" => log::Level::Error,
        "WARN" => log::Level::Warn,
        "INFO" => log::Level::Info,
        "DEBUG" => log::Level::Debug,
        "TRACE" => log::Level::Trace,
        _ => return (log::Level::Warn, line),
    };
    (level, rest.trim_start())
}

fn default_socket_path() -> PathBuf {
    // On macOS the temp dir is private to the user. Keep the name short: Unix
    // socket paths are limited to about 104 bytes there.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("md-wingman-{}-{n}.sock", std::process::id()))
}

type Routes = Arc<Mutex<HashMap<String, Sender<Event>>>>;

struct Shared {
    writer: Mutex<UnixStream>,
    routes: Routes,
    next_id: AtomicU64,
}

impl Shared {
    fn send(&self, req: &Request) -> io::Result<()> {
        let mut line = serde_json::to_vec(req).map_err(io::Error::other)?;
        line.push(b'\n');
        let mut w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        w.write_all(&line)?;
        w.flush()
    }

    fn register(&self) -> (String, Receiver<Event>) {
        let id = format!("r{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = mpsc::channel();
        self.routes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), tx);
        (id, rx)
    }

    fn unregister(&self, id: &str) {
        self.routes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }
}

/// A connection to the service. Cheap to clone; clones share the connection.
#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
}

impl Client {
    pub fn connect(socket_path: &Path) -> io::Result<Client> {
        let stream = UnixStream::connect(socket_path)
            .inspect_err(|e| log::error!("couldn't connect to {}: {e}", socket_path.display()))?;
        log::debug!("connected to {}", socket_path.display());
        let reader = stream.try_clone()?;
        let routes: Routes = Arc::default();
        thread::Builder::new()
            .name("wingman-client-reader".into())
            .spawn({
                let routes = routes.clone();
                move || read_loop(reader, routes)
            })?;
        Ok(Client {
            shared: Arc::new(Shared {
                writer: Mutex::new(stream),
                routes,
                next_id: AtomicU64::new(1),
            }),
        })
    }

    /// Checks the service is alive and speaks this protocol version.
    pub fn hello(&self) -> io::Result<ServiceInfo> {
        let (id, rx) = self.shared.register();
        self.shared.send(&Request::Hello { id: &id })?;
        let event = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no hello reply"))
            .inspect_err(|e| log::error!("handshake failed: {e}"));
        self.shared.unregister(&id);
        let result = match event? {
            Event::Hello {
                protocol,
                version,
                auth,
            } if protocol == PROTOCOL_VERSION => Ok(ServiceInfo { version, auth }),
            Event::Hello { protocol, .. } => Err(io::Error::other(format!(
                "service speaks protocol {protocol}, this app speaks {PROTOCOL_VERSION}"
            ))),
            other => Err(io::Error::other(format!(
                "unexpected reply to hello: {other:?}"
            ))),
        };
        match &result {
            Ok(info) => log::info!(
                "service {} ready, protocol {PROTOCOL_VERSION}, {} sign-in",
                info.version,
                info.auth.as_env_value()
            ),
            Err(e) => log::error!("handshake failed: {e}"),
        }
        result
    }

    /// Starts an assistant task on `text`. `context` is optional surrounding
    /// document text the assistant may use but should not edit.
    pub fn start_task(
        &self,
        task: TaskKind,
        text: &str,
        context: Option<&str>,
    ) -> io::Result<Task> {
        let (id, events) = self.shared.register();
        if let Err(e) = self.shared.send(&Request::Task {
            id: &id,
            task,
            text,
            context,
        }) {
            log::error!("couldn't send task {id}: {e}");
            self.shared.unregister(&id);
            return Err(e);
        }
        log::debug!(
            "sent task {id} ({task:?}, {} bytes, context {} bytes)",
            text.len(),
            context.map_or(0, str::len)
        );
        Ok(Task {
            id,
            events,
            shared: self.shared.clone(),
        })
    }

    /// Cancels the task with this id. Its last event will be an `Error` with
    /// the message `cancelled`.
    pub fn cancel(&self, task_id: &str) -> io::Result<()> {
        log::debug!("cancelling task {task_id}");
        self.shared.send(&Request::Cancel {
            id: &format!("{task_id}-cancel"),
            target: task_id,
        })
    }

    /// Asks the service process to exit.
    pub fn shutdown(&self) -> io::Result<()> {
        log::info!("asking the service to shut down");
        self.shared.send(&Request::Shutdown { id: "shutdown" })
    }
}

/// One running assistant task.
pub struct Task {
    id: String,
    events: Receiver<Event>,
    shared: Arc<Shared>,
}

impl Task {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The task's events, in order. The last one is `Done` or `Error`. The
    /// iterator also ends if the connection drops.
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    pub fn cancel(&self) -> io::Result<()> {
        log::debug!("cancelling task {}", self.id);
        self.shared.send(&Request::Cancel {
            id: &format!("{}-cancel", self.id),
            target: &self.id,
        })
    }

    /// Blocks until the task finishes, calling `on_event` for each event.
    /// Returns the full reply, or the service's error message.
    pub fn wait(self, mut on_event: impl FnMut(&Event)) -> Result<String, String> {
        for event in self.events.iter() {
            on_event(&event);
            match event {
                Event::Done { text, .. } => return Ok(text),
                Event::Error { message } => return Err(message),
                _ => {}
            }
        }
        Err("connection to the service was lost".into())
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.shared.unregister(&self.id);
    }
}

fn read_loop(stream: UnixStream, routes: Routes) {
    for line in BufReader::new(stream).lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                log::warn!("lost the connection to the service: {e}");
                break;
            }
        };
        let Envelope { id, event } = match serde_json::from_str::<Envelope>(&line) {
            Ok(envelope) => envelope,
            Err(e) => {
                log::warn!(
                    "ignoring a message the app doesn't understand ({e}): {} bytes",
                    line.len()
                );
                continue;
            }
        };
        log::trace!("event for {id}: {}", event_summary(&event));
        let mut routes = routes.lock().unwrap_or_else(|e| e.into_inner());
        let finished = event.is_final();
        match routes.get(&id) {
            Some(tx) => {
                let _ = tx.send(event);
            }
            None => log::debug!("no one is waiting for {id} any more"),
        }
        if finished {
            routes.remove(&id);
        }
    }
    // Connection closed: dropping the senders ends every waiting task.
    let mut routes = routes.lock().unwrap_or_else(|e| e.into_inner());
    if routes.is_empty() {
        log::debug!("connection to the service closed");
    } else {
        log::warn!(
            "connection to the service closed with {} tasks still waiting",
            routes.len()
        );
    }
    routes.clear();
}

/// An event's kind and size, without its text.
fn event_summary(event: &Event) -> String {
    match event {
        Event::Hello { protocol, .. } => format!("hello (protocol {protocol})"),
        Event::Delta { text } => format!("delta ({} bytes)", text.len()),
        Event::Status { message } => format!("status {message:?}"),
        Event::Done { text, data } => format!(
            "done ({} bytes{})",
            text.len(),
            if data.is_some() { ", with data" } else { "" }
        ),
        Event::Error { message } => format!("error {message:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::service_line;
    use log::Level;

    #[test]
    fn reads_service_log_levels() {
        assert_eq!(service_line("INFO listening"), (Level::Info, "listening"));
        assert_eq!(
            service_line("ERROR task r1 failed: boom"),
            (Level::Error, "task r1 failed: boom")
        );
        assert_eq!(service_line("DEBUG"), (Level::Debug, ""));
        assert_eq!(
            service_line("panic: something broke"),
            (Level::Warn, "panic: something broke")
        );
    }
}
