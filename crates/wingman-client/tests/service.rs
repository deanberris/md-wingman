//! End-to-end tests against the real service process, run with canned replies
//! (WINGMAN_FAKE=1) so they need no Claude login. They start the service with
//! `bun run src/main.ts`, so they need Bun and `bun install` in `service/`.
//! They are skipped when Bun is not installed.

use std::path::PathBuf;
use std::process::Command;

use wingman_client::{AuthMode, Event, Service, TaskKind};

fn fake_service() -> Option<Service> {
    fake_service_with(AuthMode::Subscription)
}

fn fake_service_with(auth: AuthMode) -> Option<Service> {
    fake_service_env(auth, &[])
}

fn fake_service_env(auth: AuthMode, env: &[(&str, &str)]) -> Option<Service> {
    if Command::new("bun").arg("--version").output().is_err() {
        eprintln!("bun not installed; skipping");
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../service");
    let mut cmd = Command::new("bun");
    cmd.current_dir(dir)
        .args(["run", "src/main.ts"])
        .env("WINGMAN_FAKE", "1")
        .envs(env.iter().copied());
    Some(Service::spawn(cmd, auth).expect("service starts"))
}

#[test]
fn hello_and_copy_edit_stream() {
    let Some(service) = fake_service() else {
        return;
    };
    let client = service.connect().unwrap();
    let info = client.hello().unwrap();
    assert_eq!(info.version, "0.1.0");
    assert_eq!(info.auth, AuthMode::Subscription);

    let task = client
        .start_task(TaskKind::CopyEdit, "teh cat sat", None)
        .unwrap();
    let mut streamed = String::new();
    let reply = task
        .wait(|e| {
            if let Event::Delta { text } = e {
                streamed.push_str(text);
            }
        })
        .unwrap();
    assert_eq!(reply, "[fake copy_edit] teh cat sat");
    assert_eq!(streamed, reply);
}

#[test]
fn concurrent_tasks_on_one_connection() {
    let Some(service) = fake_service() else {
        return;
    };
    let client = service.connect().unwrap();
    let a = client
        .start_task(TaskKind::FactCheck, "alpha", Some("doc"))
        .unwrap();
    let b = client
        .start_task(TaskKind::FindReferences, "beta", None)
        .unwrap();
    let mut saw_status = false;
    let ra = a
        .wait(|e| saw_status |= matches!(e, Event::Status { .. }))
        .unwrap();
    let rb = b.wait(|_| {}).unwrap();
    assert!(saw_status);
    assert_eq!(ra, "[fake fact_check] alpha");
    assert_eq!(rb, "[fake find_references] beta");
}

#[test]
fn service_stops_when_dropped() {
    let Some(service) = fake_service() else {
        return;
    };
    let path = service.socket_path().to_owned();
    drop(service);
    assert!(!path.exists(), "socket file should be removed");
}

#[test]
fn reports_api_key_mode() {
    let Some(service) = fake_service_with(AuthMode::ApiKey) else {
        return;
    };
    let info = service.connect().unwrap().hello().unwrap();
    assert_eq!(info.auth, AuthMode::ApiKey);
}

#[test]
fn reports_the_model_provider() {
    let Some(service) = fake_service_env(
        AuthMode::Provider,
        &[
            ("WINGMAN_PROVIDER_URL", "http://localhost:11434"),
            ("WINGMAN_PROVIDER_NAME", "Ollama"),
            ("WINGMAN_MODEL", "qwen3"),
        ],
    ) else {
        return;
    };
    let info = service.connect().unwrap().hello().unwrap();
    assert_eq!(info.auth, AuthMode::Provider);
    assert_eq!(info.provider.as_deref(), Some("Ollama"));
    assert_eq!(info.model.as_deref(), Some("qwen3"));
}

#[test]
fn the_services_log_lines_come_back() {
    let Some(service) = fake_service() else {
        return;
    };
    let client = service.connect().unwrap();
    client.hello().unwrap();
    let task = client
        .start_task(TaskKind::CopyEdit, "a secret passage", None)
        .unwrap();
    task.wait(|_| {}).unwrap();
    // The log thread may be a moment behind the socket.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let lines = loop {
        let lines = service.recent_lines();
        if lines.iter().any(|l| l.contains("done in")) || std::time::Instant::now() > deadline {
            break lines;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        lines.iter().any(|l| l.contains("listening on")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("copy_edit started")),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("done in")), "{lines:?}");
    // Document text stays out of the log by default.
    assert!(
        !lines.iter().any(|l| l.contains("secret passage")),
        "{lines:?}"
    );
}

/// Collects log records, to see what the client reports.
struct Capture(std::sync::Mutex<Vec<(String, String)>>);

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        self.0
            .lock()
            .unwrap()
            .push((record.target().to_string(), record.args().to_string()));
    }
    fn flush(&self) {}
}

static CAPTURE: Capture = Capture(std::sync::Mutex::new(Vec::new()));

#[test]
fn a_service_that_dies_is_reported_as_a_crash() {
    let _ = log::set_logger(&CAPTURE);
    log::set_max_level(log::LevelFilter::Info);
    let Some(service) = fake_service() else {
        return;
    };
    service.connect().unwrap().hello().unwrap();
    Command::new("kill")
        .args(["-9", &service.pid().to_string()])
        .status()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let crash = loop {
        let crash = CAPTURE
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|(target, _)| target == "crash")
            .map(|(_, m)| m.clone());
        if crash.is_some() || std::time::Instant::now() > deadline {
            break crash;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let crash = crash.expect("a crash record");
    assert!(crash.contains("stopped unexpectedly"), "{crash}");
    assert!(crash.contains("listening on"), "{crash}");
}
