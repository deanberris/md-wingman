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
    if Command::new("bun").arg("--version").output().is_err() {
        eprintln!("bun not installed; skipping");
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../service");
    let mut cmd = Command::new("bun");
    cmd.current_dir(dir)
        .args(["run", "src/main.ts"])
        .env("WINGMAN_FAKE", "1");
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
