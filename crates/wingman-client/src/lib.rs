//! Client for the md-wingman assistant service.
//!
//! The service is a separate process (see `service/` in the repository) that
//! talks to Claude through the Claude Agent SDK. This crate starts it, connects
//! to it over a Unix socket, and streams replies back. It uses plain threads and
//! channels so it fits any UI toolkit's event loop.
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

/// A running service process. Dropping it stops the process.
pub struct Service {
    child: Child,
    socket_path: PathBuf,
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
        let child = command
            .env("WINGMAN_AUTH", auth.as_env_value())
            .arg("--socket")
            .arg(&socket_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()?;
        let mut service = Service { child, socket_path };
        service.wait_until_listening()?;
        Ok(service)
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
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
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
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
        let stream = UnixStream::connect(socket_path)?;
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
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no hello reply"));
        self.shared.unregister(&id);
        match event? {
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
        }
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
            self.shared.unregister(&id);
            return Err(e);
        }
        Ok(Task {
            id,
            events,
            shared: self.shared.clone(),
        })
    }

    /// Cancels the task with this id. Its last event will be an `Error` with
    /// the message `cancelled`.
    pub fn cancel(&self, task_id: &str) -> io::Result<()> {
        self.shared.send(&Request::Cancel {
            id: &format!("{task_id}-cancel"),
            target: task_id,
        })
    }

    /// Asks the service process to exit.
    pub fn shutdown(&self) -> io::Result<()> {
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
                Event::Done { text } => return Ok(text),
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
        let Ok(line) = line else { break };
        let Ok(Envelope { id, event }) = serde_json::from_str::<Envelope>(&line) else {
            continue;
        };
        let mut routes = routes.lock().unwrap_or_else(|e| e.into_inner());
        let finished = event.is_final();
        if let Some(tx) = routes.get(&id) {
            let _ = tx.send(event);
        }
        if finished {
            routes.remove(&id);
        }
    }
    // Connection closed: dropping the senders ends every waiting task.
    routes.lock().unwrap_or_else(|e| e.into_inner()).clear();
}
