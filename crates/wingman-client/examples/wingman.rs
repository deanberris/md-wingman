//! Try the assistant service from the command line.
//!
//!     cargo run -p wingman-client --example wingman -- copy-edit "Their going to the libary."
//!     cargo run -p wingman-client --example wingman -- fact-check "The Eiffel Tower opened in 1899."
//!     cargo run -p wingman-client --example wingman -- references "Coffee improves memory."
//!
//! Build the service first (`cd service && bun install && bun run build`), or
//! point WINGMAN_SERVICE at the executable. Set WINGMAN_FAKE=1 for canned
//! replies without calling Claude.
//!
//! By default the service uses your Claude subscription (your `claude` login)
//! and ignores any ANTHROPIC_API_KEY. Pass --api-key first to use the key and
//! bill the Claude API instead.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use wingman_client::{AuthMode, Event, Service, TaskKind};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let auth = if args.first().map(String::as_str) == Some("--api-key") {
        args.remove(0);
        AuthMode::ApiKey
    } else {
        AuthMode::Subscription
    };
    let (task, text) = match args.as_slice() {
        [kind, text @ ..] if !text.is_empty() => {
            let task = match kind.as_str() {
                "copy-edit" => TaskKind::CopyEdit,
                "fact-check" => TaskKind::FactCheck,
                "references" => TaskKind::FindReferences,
                _ => return usage(),
            };
            (task, text.join(" "))
        }
        _ => return usage(),
    };

    let exe = std::env::var_os("WINGMAN_SERVICE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../service/dist/wingman-service")
        });
    let service = match Service::spawn(Command::new(&exe), auth) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not start {}: {e}", exe.display());
            return ExitCode::FAILURE;
        }
    };
    let client = match service.connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not connect: {e}");
            return ExitCode::FAILURE;
        }
    };
    match client.hello() {
        Ok(info) => eprintln!(
            "connected to wingman-service {} (sign-in: {})",
            info.version,
            info.auth.as_env_value()
        ),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    let task = match client.start_task(task, &text, None) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("could not send request: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = task.wait(|event| match event {
        Event::Delta { text } => {
            print!("{text}");
            let _ = std::io::stdout().flush();
        }
        Event::Status { message } => eprintln!("\n[{message}]"),
        Event::Done {
            data: Some(data), ..
        } => {
            println!(
                "\n{}",
                serde_json::to_string_pretty(data).unwrap_or_default()
            );
        }
        _ => {}
    });
    println!();
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: wingman [--api-key] <copy-edit|fact-check|references> <text>");
    ExitCode::from(2)
}
