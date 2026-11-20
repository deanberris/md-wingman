//! End-to-end tests against the real service process, run with canned replies
//! (WINGMAN_FAKE=1) so they need no Claude login. They start the service with
//! `bun run src/main.ts`, so they need Bun and `bun install` in `service/`.
//! They are skipped when Bun is not installed.

use std::path::PathBuf;
use std::process::Command;

use wingman_client::{Event, Service, TaskKind};

fn fake_service() -> Option<Service> {
    if Command::new("bun").arg("--version").output().is_err() {
        eprintln!("bun not installed; skipping");
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../service");
    let mut cmd = Command::new("bun");
    cmd.current_dir(dir)
        .args(["run", "src/main.ts"])
        .env("WINGMAN_FAKE", "1");
    Some(Service::spawn(cmd).expect("service starts"))
}

#[test]
fn hello_and_copy_edit_stream() {
    let Some(service) = fake_service() else {
        return;
    };
    let client = service.connect().unwrap();
    assert_eq!(client.hello().unwrap(), "0.1.0");

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
