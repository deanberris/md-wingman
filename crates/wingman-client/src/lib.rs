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
//! sets up gets one story from both processes. A crash in the service (a
//! `FATAL` line with its stack trace, or the process ending while it should
//! be running) is logged under the `crash` target, which md-wingman's logger
//! writes up as a crash report.
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
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub use protocol::{AuthMode, Event, PROTOCOL_VERSION, TaskKind};
use protocol::{Envelope, Request};

/// What the service reports about itself in the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    pub version: String,
    /// The credential the service is using, so the UI can show it.
    pub auth: AuthMode,
    /// In provider mode: the provider's name and the model in use.
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// How long [`Service::spawn`] waits for the service to start listening.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// How many of the service's last log lines are kept, to explain a failed
/// start.
const KEPT_LINES: usize = 20;

/// The log target for crashes (see `wingman_log::crash::TARGET`).
const CRASH_TARGET: &str = "crash";

/// Where the service is in its life, so its end can be told apart from a crash.
const STARTING: u8 = 0;
const RUNNING: u8 = 1;
const STOPPING: u8 = 2;

/// A running service process. Dropping it stops the process.
pub struct Service {
    child: Child,
    socket_path: PathBuf,
    /// The service's last few stderr lines.
    recent: Arc<Mutex<Vec<String>>>,
    /// `STARTING`, `RUNNING` or `STOPPING`.
    phase: Arc<AtomicU8>,
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
        let phase = Arc::new(AtomicU8::new(STARTING));
        if let Some(stderr) = child.stderr.take() {
            let (recent, phase) = (recent.clone(), phase.clone());
            thread::Builder::new()
                .name("wingman-service-log".into())
                .spawn(move || forward_log(stderr, &recent, &phase))?;
        }
        log::debug!("service pid {}", child.id());
        let mut service = Service {
            child,
            socket_path,
            recent,
            phase,
        };
        match service.wait_until_listening() {
            Ok(()) => {
                service.phase.store(RUNNING, Ordering::SeqCst);
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

    /// The service process's id.
    pub fn pid(&self) -> u32 {
        self.child.id()
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
        self.phase.store(STOPPING, Ordering::SeqCst);
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
/// last few for error messages, and reports a crash.
fn forward_log(stderr: impl io::Read, recent: &Mutex<Vec<String>>, phase: &AtomicU8) {
    let mut crashed = false;
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let line = service_line(&line);
        if line.fatal {
            crashed = true;
            log::error!(target: CRASH_TARGET, "{}", line.message);
        } else {
            log::log!(target: "service", line.level, "{}", line.message);
        }
        let mut recent = recent.lock().unwrap_or_else(|e| e.into_inner());
        if recent.len() == KEPT_LINES {
            recent.remove(0);
        }
        recent.push(line.message);
    }
    // The service's stderr closes when it exits. While it should be running,
    // that's a crash, even without a FATAL line (killed, out of memory, or
    // a crash in Bun itself).
    if phase.load(Ordering::SeqCst) == RUNNING && !crashed {
        let last = recent.lock().unwrap_or_else(|e| e.into_inner()).join("\n");
        log::error!(
            target: CRASH_TARGET,
            "wingman-service stopped unexpectedly.\n\nIts last log lines:\n{last}"
        );
    } else {
        log::debug!("service closed its log");
    }
}

/// One line of the service's log, read back.
#[derive(Debug, PartialEq, Eq)]
pub struct ServiceLine {
    pub level: log::Level,
    /// A crash, with its stack trace.
    pub fatal: bool,
    /// With its newlines put back.
    pub message: String,
}

/// Reads a service log line, `"INFO message"`. Lines without a level (such
/// as Bun's own output) are warnings. The service writes newlines in a
/// message as `\n` and backslashes as `\\`.
pub fn service_line(line: &str) -> ServiceLine {
    let (first, rest) = line.split_once(' ').unwrap_or((line, ""));
    let (level, fatal) = match first {
        "FATAL" => (log::Level::Error, true),
        "ERROR" => (log::Level::Error, false),
        "WARN" => (log::Level::Warn, false),
        "INFO" => (log::Level::Info, false),
        "DEBUG" => (log::Level::Debug, false),
        "TRACE" => (log::Level::Trace, false),
        _ => {
            return ServiceLine {
                level: log::Level::Warn,
                fatal: false,
                message: line.to_string(),
            };
        }
    };
    ServiceLine {
        level,
        fatal,
        message: unescape(rest.trim_start()),
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
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
                provider,
                model,
            } if protocol == PROTOCOL_VERSION => Ok(ServiceInfo {
                version,
                auth,
                provider,
                model,
            }),
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
    use super::{ServiceLine, service_line};
    use log::Level;

    fn line(level: Level, fatal: bool, message: &str) -> ServiceLine {
        ServiceLine {
            level,
            fatal,
            message: message.into(),
        }
    }

    #[test]
    fn reads_service_log_levels() {
        assert_eq!(
            service_line("INFO listening"),
            line(Level::Info, false, "listening")
        );
        assert_eq!(
            service_line("ERROR task r1 failed: boom"),
            line(Level::Error, false, "task r1 failed: boom")
        );
        assert_eq!(service_line("DEBUG"), line(Level::Debug, false, ""));
        assert_eq!(
            service_line("panic: something broke"),
            line(Level::Warn, false, "panic: something broke")
        );
    }

    #[test]
    fn puts_a_stack_trace_back_together() {
        assert_eq!(
            service_line(
                r"FATAL crashed: Error: boom
    at main (C:\x\main.ts:1:1)"
            ),
            line(
                Level::Error,
                true,
                "crashed: Error: boom\n    at main (C:\\x\\main.ts:1:1)"
            )
        );
    }
}
