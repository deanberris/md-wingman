//! md-wingman's log: one file a writer can attach to a bug report.
//!
//! The app and the assistant service both log here (the app forwards the
//! service's stderr), so one file tells the whole story. On macOS it is
//! `~/Library/Logs/md-wingman/md-wingman.log`, which Console.app also shows.
//!
//! - Levels come from the `log` crate. `WINGMAN_LOG` sets the level (`error`,
//!   `warn`, `info`, `debug` or `trace`); the default is `info`.
//! - Secrets never reach the file: anything passed to [`add_secret`] is
//!   replaced with `[redacted]`, and so is anything that looks like an API
//!   key (`sk-…`) or a bearer token, in case one slips into an error message.
//! - Document text is left out unless `WINGMAN_LOG_TEXT=1`: log it with
//!   [`text`], which otherwise shows only its length and a fingerprint.
//! - The file rotates by itself once it reaches [`MAX_BYTES`], at launch or
//!   while running; the last [`KEEP_ROTATED`] old ones are kept beside it as
//!   `md-wingman.log.1` (the newest) to `.3`.
//! - A crash (a panic in the app, or the assistant service dying) writes a
//!   report with its stack trace and the log lines before it to `crashes/`
//!   beside the log. See [`crash`].

pub mod crash;

use std::collections::VecDeque;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub use log::{Level, LevelFilter};

/// How big the log file grows before it is rotated.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// How many rotated logs are kept.
pub const KEEP_ROTATED: usize = 3;

/// How many recent lines are kept in memory, for crash reports.
const RECENT_LINES: usize = 300;

/// The longest stretch of document text logged, with `WINGMAN_LOG_TEXT=1`.
const MAX_TEXT_CHARS: usize = 2_000;

/// Secrets shorter than this are not scrubbed: they would match ordinary words.
const MIN_SECRET_LEN: usize = 8;

static SECRETS: RwLock<Vec<String>> = RwLock::new(Vec::new());
static INCLUDE_TEXT: AtomicBool = AtomicBool::new(false);
static PATH: OnceLock<PathBuf> = OnceLock::new();
static APP: OnceLock<String> = OnceLock::new();
static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// How to set up logging.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub path: PathBuf,
    pub level: LevelFilter,
    /// Log document text, not just its size. Off by default.
    pub include_text: bool,
    /// Also write to stderr: on when it's a terminal, as with `scripts/dev.sh`.
    pub echo_stderr: bool,
    /// The app, its version and the system, for the top of crash reports.
    pub app: String,
}

impl Config {
    /// The defaults, adjusted by `WINGMAN_LOG`, `WINGMAN_LOG_TEXT` and
    /// `WINGMAN_LOG_FILE`.
    pub fn from_env() -> Config {
        Config {
            path: std::env::var_os("WINGMAN_LOG_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(default_path),
            level: std::env::var("WINGMAN_LOG")
                .ok()
                .and_then(|v| parse_level(&v))
                .unwrap_or(LevelFilter::Info),
            include_text: std::env::var("WINGMAN_LOG_TEXT").is_ok_and(|v| v == "1"),
            echo_stderr: io::stderr().is_terminal(),
            app: String::new(),
        }
    }
}

/// Where the log goes unless `WINGMAN_LOG_FILE` says otherwise.
pub fn default_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    if cfg!(target_os = "macos") {
        home.join("Library/Logs/md-wingman/md-wingman.log")
    } else {
        dirs::state_dir()
            .or_else(dirs::data_local_dir)
            .unwrap_or(home)
            .join("md-wingman/md-wingman.log")
    }
}

/// `"debug"` and so on, as used by `WINGMAN_LOG`.
pub fn parse_level(value: &str) -> Option<LevelFilter> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

/// Starts logging to the file. Call once, early; later calls fail.
pub fn init(config: Config) -> io::Result<()> {
    if let Some(dir) = config.path.parent() {
        fs::create_dir_all(dir)?;
    }
    rotate_if_big(&config.path, 0)?;
    let file = open(&config.path)?;
    let written = file.metadata().map(|m| m.len()).unwrap_or(0);
    INCLUDE_TEXT.store(config.include_text, Ordering::Relaxed);
    let logger = FileLogger {
        path: config.path.clone(),
        echo_stderr: config.echo_stderr,
        state: Mutex::new(FileState {
            file: Some(file),
            written,
        }),
    };
    log::set_boxed_logger(Box::new(logger)).map_err(io::Error::other)?;
    log::set_max_level(config.level);
    let _ = APP.set(config.app);
    let _ = PATH.set(config.path);
    Ok(())
}

/// The log file, once [`init`] has run.
pub fn path() -> Option<&'static Path> {
    PATH.get().map(PathBuf::as_path)
}

/// Changes the level while running, as for Help ▸ Verbose Logging.
pub fn set_level(level: LevelFilter) {
    log::set_max_level(level);
}

pub fn level() -> LevelFilter {
    log::max_level()
}

/// Keeps `secret` out of the log from now on.
pub fn add_secret(secret: &str) {
    let secret = secret.trim();
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    let mut secrets = SECRETS.write().unwrap_or_else(|e| e.into_inner());
    if !secrets.iter().any(|s| s == secret) {
        secrets.push(secret.to_string());
    }
}

/// Stops scrubbing `secret`, once it is no longer used.
pub fn remove_secret(secret: &str) {
    SECRETS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|s| s != secret.trim());
}

/// Whether document text is logged (`WINGMAN_LOG_TEXT=1`).
pub fn includes_text() -> bool {
    INCLUDE_TEXT.load(Ordering::Relaxed)
}

/// Document text for a log line: its size and fingerprint, plus the text
/// itself only with `WINGMAN_LOG_TEXT=1`.
pub fn text(text: &str) -> Text<'_> {
    Text {
        text,
        include: includes_text(),
    }
}

/// See [`text`].
pub struct Text<'a> {
    text: &'a str,
    include: bool,
}

impl fmt::Display for Text<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let chars = self.text.chars().count();
        let words = self.text.split_whitespace().count();
        write!(
            f,
            "[{chars} chars, {words} words, #{:08x}]",
            fingerprint(self.text)
        )?;
        if self.include {
            let shown: String = self.text.chars().take(MAX_TEXT_CHARS).collect();
            write!(f, " {shown:?}")?;
            if chars > MAX_TEXT_CHARS {
                write!(f, "…")?;
            }
        }
        Ok(())
    }
}

/// A short, stable fingerprint, so two log lines about the same paragraph
/// can be matched up without showing it. FNV-1a.
pub fn fingerprint(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    })
}

/// The app description given to [`init`].
pub fn app() -> &'static str {
    APP.get().map_or("", String::as_str)
}

/// The last few hundred lines logged, oldest first, already scrubbed.
pub fn recent_lines() -> Vec<String> {
    RECENT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}

/// Logs panics, with where they happened and a backtrace, and writes a
/// crash report, before the default handler runs.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        log::error!(
            target: crash::TARGET,
            "md-wingman panicked in thread '{thread}' at {location}: {message}\n\nStack trace:\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        log::logger().flush();
        previous(info);
    }));
}

/// Replaces secrets, API keys and bearer tokens with `[redacted]`.
pub fn scrub(line: &str) -> String {
    let secrets = SECRETS.read().unwrap_or_else(|e| e.into_inner());
    scrub_with(line, &secrets)
}

fn scrub_with(line: &str, secrets: &[String]) -> String {
    let mut out = line.to_string();
    for secret in secrets {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), "[redacted]");
        }
    }
    let out = scrub_token_after(&out, "sk-", 16);
    scrub_token_after(&out, "Bearer ", 8)
}

/// Redacts a run of at least `min` token characters following `prefix`.
fn scrub_token_after(line: &str, prefix: &str, min: usize) -> String {
    let is_token = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '=');
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(prefix) {
        let after = &rest[at + prefix.len()..];
        let len = after.find(|c: char| !is_token(c)).unwrap_or(after.len());
        out.push_str(&rest[..at + prefix.len()]);
        if len >= min {
            out.push_str("[redacted]");
        } else {
            out.push_str(&after[..len]);
        }
        rest = &after[len..];
    }
    out.push_str(rest);
    out
}

struct FileState {
    file: Option<File>,
    written: u64,
}

struct FileLogger {
    path: PathBuf,
    echo_stderr: bool,
    state: Mutex<FileState>,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(
            SystemTime::now(),
            record.level(),
            record.target(),
            &record.args().to_string(),
        );
        let mut line = scrub(&line);
        if record.target() == crash::TARGET {
            // The report has the crash and the lines before it.
            let message = scrub(&record.args().to_string());
            let note = match crash::write_report(&message) {
                Ok(path) => format!(
                    "    crash report: {}
",
                    path.display()
                ),
                Err(e) => format!(
                    "    couldn't write a crash report: {e}
"
                ),
            };
            line.push_str(&note);
        }
        if self.echo_stderr {
            let _ = io::stderr().write_all(line.as_bytes());
        }
        {
            let mut recent = RECENT.lock().unwrap_or_else(|e| e.into_inner());
            if recent.len() == RECENT_LINES {
                recent.pop_front();
            }
            recent.push_back(line.trim_end().to_string());
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.written + line.len() as u64 > MAX_BYTES {
            // Close, rotate and reopen. If that fails, keep the old file.
            state.file = None;
            let _ = rotate_if_big(&self.path, 0);
            state.file = open(&self.path).ok();
            state.written = 0;
        }
        if let Some(file) = state.file.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                state.written += line.len() as u64;
            }
        }
    }

    fn flush(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = state.file.as_mut() {
            let _ = file.flush();
        }
    }
}

fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Once the log is over `MAX_BYTES - headroom`, moves it aside as `.1`,
/// shifting older ones along and dropping the oldest.
fn rotate_if_big(path: &Path, headroom: u64) -> io::Result<()> {
    match fs::metadata(path) {
        Ok(m) if m.len() + headroom > MAX_BYTES => {
            for n in (1..KEEP_ROTATED).rev() {
                let from = rotated_n(path, n);
                if from.exists() {
                    fs::rename(&from, rotated_n(path, n + 1))?;
                }
            }
            fs::rename(path, rotated(path))
        }
        _ => Ok(()),
    }
}

/// `md-wingman.log.1`, the newest rotated log, beside the log.
pub fn rotated(path: &Path) -> PathBuf {
    rotated_n(path, 1)
}

fn rotated_n(path: &Path, n: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{n}"));
    path.with_file_name(name)
}

/// `2026-11-21T10:25:27.123Z INFO  target: message`, one line per record
/// (continuation lines are indented, so a backtrace stays together).
fn format_line(time: SystemTime, level: Level, target: &str, message: &str) -> String {
    let message = message.trim_end().replace('\n', "\n    ");
    format!("{} {level:<5} {target}: {message}\n", timestamp(time))
}

/// An ISO 8601 UTC timestamp with milliseconds.
fn timestamp(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60,
        since.subsec_millis()
    )
}

/// Days since 1970-01-01 to a date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_lines_with_utc_timestamps() {
        let t = UNIX_EPOCH + Duration::from_millis(1_795_256_727_123);
        assert_eq!(timestamp(t), "2026-11-21T10:25:27.123Z");
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_line(t, Level::Warn, "jev", "first\nsecond\n"),
            "2026-11-21T10:25:27.123Z WARN  jev: first\n    second\n"
        );
    }

    #[test]
    fn scrubs_known_secrets_keys_and_tokens() {
        let secrets = vec!["ts_live_0123456789".to_string()];
        let line = "key ts_live_0123456789 then sk-ant-api03-abcdefghijklmnopqrstuvwxyz and \
                    Authorization: Bearer abc.def-ghi_jkl but sk-short and Bearer me";
        let out = scrub_with(line, &secrets);
        assert!(!out.contains("ts_live_0123456789"), "{out}");
        assert!(!out.contains("abcdefghijklmnop"), "{out}");
        assert!(!out.contains("abc.def-ghi_jkl"), "{out}");
        assert!(
            out.contains("key [redacted] then sk-[redacted] and"),
            "{out}"
        );
        assert!(out.contains("Bearer [redacted]"), "{out}");
        // Ordinary words that happen to follow the prefixes stay.
        assert!(out.ends_with("but sk-short and Bearer me"), "{out}");
    }

    #[test]
    fn ignores_secrets_too_short_to_be_safe() {
        add_secret("abc");
        assert_eq!(scrub("abc abc"), "abc abc");
        add_secret("a-long-test-secret");
        assert_eq!(scrub("x a-long-test-secret y"), "x [redacted] y");
        remove_secret("a-long-test-secret");
        assert_eq!(scrub("x a-long-test-secret y"), "x a-long-test-secret y");
    }

    #[test]
    fn text_is_left_out_unless_asked_for() {
        let hidden = Text {
            text: "Their going home.",
            include: false,
        }
        .to_string();
        assert!(hidden.starts_with("[17 chars, 3 words, #"), "{hidden}");
        assert!(!hidden.contains("going"), "{hidden}");
        let shown = Text {
            text: "Their going home.",
            include: true,
        }
        .to_string();
        assert!(shown.ends_with(" \"Their going home.\""), "{shown}");
        // The same text always has the same fingerprint.
        assert_eq!(fingerprint("abc"), fingerprint("abc"));
        assert_ne!(fingerprint("abc"), fingerprint("abd"));
    }

    #[test]
    fn parses_levels() {
        assert_eq!(parse_level("DEBUG"), Some(LevelFilter::Debug));
        assert_eq!(parse_level(" warning "), Some(LevelFilter::Warn));
        assert_eq!(parse_level("loud"), None);
    }

    #[test]
    fn rotates_a_big_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("md-wingman.log");
        // Five rotations keep only the last three, newest as .1.
        for n in 1..=5u8 {
            fs::write(&path, vec![b'0' + n; MAX_BYTES as usize + 1]).unwrap();
            rotate_if_big(&path, 0).unwrap();
        }
        assert!(!path.exists());
        assert_eq!(rotated(&path).file_name().unwrap(), "md-wingman.log.1");
        let first = |n| fs::read(rotated_n(&path, n)).unwrap()[0];
        assert_eq!((first(1), first(2), first(3)), (b'5', b'4', b'3'));
        assert!(!rotated_n(&path, 4).exists());
        // A small one stays.
        fs::write(&path, "small").unwrap();
        rotate_if_big(&path, 0).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn writes_scrubbed_lines_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/md-wingman.log");
        init(Config {
            path: path.clone(),
            level: LevelFilter::Info,
            include_text: false,
            echo_stderr: false,
            app: "md-wingman test".into(),
        })
        .unwrap();
        assert_eq!(super::path(), Some(path.as_path()));
        add_secret("file-test-secret-value");
        log::info!(target: "test", "hello with file-test-secret-value");
        log::debug!(target: "test", "not at info");
        log::logger().flush();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(
            contents.contains("INFO  test: hello with [redacted]"),
            "{contents}"
        );
        assert!(!contents.contains("not at info"), "{contents}");
        set_level(LevelFilter::Debug);
        log::debug!(target: "test", "now at debug");
        log::logger().flush();
        assert!(fs::read_to_string(&path).unwrap().contains("now at debug"));

        // A crash writes a report with the lines before it.
        log::error!(target: crash::TARGET, "it broke\n\nStack trace:\n  0: main");
        let reports = crash::reports_in(&crash::dir_for(&path));
        assert_eq!(reports.len(), 1, "{reports:?}");
        let report = fs::read_to_string(&reports[0]).unwrap();
        assert!(report.contains("md-wingman test"), "{report}");
        assert!(report.contains("it broke"), "{report}");
        assert!(report.contains("  0: main"), "{report}");
        assert!(report.contains("now at debug"), "{report}");
        assert!(report.contains("hello with [redacted]"), "{report}");
        // Seen once at the next launch, then not again.
        assert_eq!(crash::unseen_in(&crash::dir_for(&path), &[]).len(), 1);
        assert!(crash::unseen_in(&crash::dir_for(&path), &[]).is_empty());
    }
}
