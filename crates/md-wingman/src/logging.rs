//! Setting up the log file and crash reports, and the Help menu's Report a
//! Problem, Show Log File, Show Crash Reports and Verbose Logging. See the
//! `wingman-log` crate for where the files are and what is kept out of them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use gpui_kit::*;
use wingman_log::LevelFilter;

use crate::settings::Settings;
use crate::workspace::Workspace;
use crate::{ReportProblem, ShowCrashReports, ShowLogFile, ToggleVerboseLogging};

/// `WINGMAN_LOG` was set, so it decides the level, not the settings.
static LEVEL_FROM_ENV: AtomicBool = AtomicBool::new(false);

/// Crash reports from earlier runs, found at launch, for the status bar.
static UNSEEN_CRASHES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Settings that change how the app behaves, logged at startup. Their
/// values are safe to log; secrets are only said to be set or not.
const ENV_VARS: [&str; 9] = [
    "WINGMAN_AUTH",
    "WINGMAN_FAKE",
    "WINGMAN_SERVICE",
    "WINGMAN_SETTINGS",
    "WINGMAN_MODEL",
    "WINGMAN_CLAUDE_PATH",
    "WINGMAN_LOG",
    "WINGMAN_LOG_TEXT",
    "WINGMAN_LOG_FILE",
];
const SECRET_ENV_VARS: [&str; 3] = [
    "WINGMAN_JEV_KEY",
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
];

/// Opens the log file and notes what the app is running on. Call first.
pub fn init() {
    let mut config = wingman_log::Config::from_env();
    config.app = app_description();
    LEVEL_FROM_ENV.store(std::env::var_os("WINGMAN_LOG").is_some(), Ordering::Relaxed);
    let path = config.path.clone();
    if let Err(e) = wingman_log::init(config) {
        eprintln!("md-wingman: couldn't open the log {}: {e}", path.display());
    }
    wingman_log::install_panic_hook();
    for name in SECRET_ENV_VARS {
        if let Ok(value) = std::env::var(name) {
            wingman_log::add_secret(&value);
        }
    }

    log::info!(
        "md-wingman {} starting: pid {}, {} {} ({}), log level {}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        std::env::consts::OS,
        os_version(),
        std::env::consts::ARCH,
        wingman_log::level(),
    );
    log::info!("log file: {}", path.display());
    if wingman_log::includes_text() {
        log::warn!("WINGMAN_LOG_TEXT=1: document text will be written to the log");
    }
    for name in ENV_VARS {
        if let Ok(value) = std::env::var(name) {
            log::info!("{name}={value}");
        }
    }
    for name in SECRET_ENV_VARS {
        if std::env::var_os(name).is_some() {
            log::info!("{name} is set");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        log::debug!("executable: {}", exe.display());
    }

    let crashes = wingman_log::crash::unseen();
    for report in &crashes {
        log::warn!("crash report from an earlier run: {}", report.display());
    }
    *UNSEEN_CRASHES.lock().unwrap_or_else(|e| e.into_inner()) = crashes;
}

/// "md-wingman 0.1.0 on macos 15.1 (aarch64)", for crash reports and issues.
fn app_description() -> String {
    format!(
        "md-wingman {} on {} {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        os_version(),
        std::env::consts::ARCH
    )
}

/// A note for the status bar if md-wingman crashed since it last ran.
pub fn take_crash_notice() -> Option<String> {
    let crashes = std::mem::take(&mut *UNSEEN_CRASHES.lock().unwrap_or_else(|e| e.into_inner()));
    match crashes.len() {
        0 => None,
        1 => Some(
            "md-wingman crashed last time. Help ▸ Report a Problem… to send the crash report"
                .into(),
        ),
        n => Some(format!(
            "md-wingman crashed {n} times since it last ran. Help ▸ Report a Problem… to send the crash reports"
        )),
    }
}

/// Shows `path` in Finder (selected), or opens it on other systems.
fn reveal(path: &Path) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    command.arg(path).spawn().map(drop)
}

/// The newest crash report, if there is one.
fn latest_crash_report() -> Option<PathBuf> {
    wingman_log::crash::dir()
        .map(|d| wingman_log::crash::reports_in(&d))
        .and_then(|mut r| r.pop())
}

/// A new-issue link with the version and system filled in. The log itself
/// is never put in the link: the writer attaches it, after a look.
pub fn report_url(app: &str, log: Option<&Path>, crash: Option<&Path>) -> String {
    let mut body = format!(
        "**What happened**\n\n\n\n**What you expected**\n\n\n\n**Steps to reproduce**\n\n1. \n\n---\n{app}\n"
    );
    if let Some(log) = log {
        body.push_str(&format!(
            "\nPlease attach the log (Help ▸ Show Log File): `{}`\n",
            log.display()
        ));
    }
    if let Some(crash) = crash {
        body.push_str(&format!(
            "\nAnd the latest crash report (Help ▸ Show Crash Reports): `{}`\n",
            crash.display()
        ));
    }
    format!(
        "{}?title={}&body={}",
        wingman_log::crash::ISSUES_URL,
        percent_encode(if crash.is_some() { "Crash: " } else { "" }),
        percent_encode(&body)
    )
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The macOS version, such as "15.1", or "" elsewhere.
fn os_version() -> String {
    if !cfg!(target_os = "macos") {
        return String::new();
    }
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|v| v.trim().to_string())
        .unwrap_or_default()
}

/// The level the settings ask for, unless `WINGMAN_LOG` set one.
pub fn apply_settings(settings: &Settings) {
    if LEVEL_FROM_ENV.load(Ordering::Relaxed) {
        return;
    }
    wingman_log::set_level(if settings.verbose_logging {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    });
}

/// The settings that shape what the assistant does, for the log.
pub fn log_settings(s: &Settings) {
    log::info!(
        "settings: continuous {} (pause {}ms, fact-check {}), Jev pre-filter {} (threshold {:.0}%, model {}), verbose logging {}",
        s.continuous,
        s.continuous_delay_ms,
        s.continuous_fact_check,
        s.jev_prefilter,
        s.jev_threshold * 100.,
        s.jev_model,
        s.verbose_logging,
    );
    log::debug!("all settings: {s:?}");
}

impl Workspace {
    pub(crate) fn on_show_log_file(
        &mut self,
        _: &ShowLogFile,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = wingman_log::path() else {
            self.set_notice("Logging isn't working; see the terminal for why", cx);
            return;
        };
        log::logger().flush();
        // Reveal it in Finder, so it can be attached to a bug report.
        match reveal(path) {
            Ok(_) => self.set_notice(format!("Log file: {}", path.display()), cx),
            Err(e) => self.set_notice(format!("Log file: {} ({e})", path.display()), cx),
        }
    }

    pub(crate) fn on_show_crash_reports(
        &mut self,
        _: &ShowCrashReports,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(latest) = latest_crash_report() else {
            self.set_notice("No crash reports: md-wingman hasn't crashed", cx);
            return;
        };
        match reveal(&latest) {
            Ok(()) => self.set_notice(
                format!(
                    "Crash reports: {}",
                    latest.parent().unwrap_or(&latest).display()
                ),
                cx,
            ),
            Err(e) => self.set_notice(format!("Crash report: {} ({e})", latest.display()), cx),
        }
    }

    pub(crate) fn on_report_problem(
        &mut self,
        _: &ReportProblem,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::logger().flush();
        let crash = latest_crash_report();
        let url = report_url(wingman_log::app(), wingman_log::path(), crash.as_deref());
        log::info!("opening a new issue on GitHub");
        cx.open_url(&url);
        // Show what to attach, so it's at hand.
        if let Some(path) = crash.as_deref().or(wingman_log::path()) {
            let _ = reveal(path);
        }
        self.set_notice(
            "Opened a new issue on GitHub. Attach the log file shown in Finder",
            cx,
        );
    }

    pub(crate) fn on_toggle_verbose_logging(
        &mut self,
        _: &ToggleVerboseLogging,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.verbose_logging = !self.settings.verbose_logging;
        self.save_settings(cx);
        apply_settings(&self.settings);
        let notice = if LEVEL_FROM_ENV.load(Ordering::Relaxed) {
            format!(
                "WINGMAN_LOG sets the log level ({}), so this applies next time it isn't set",
                wingman_log::level()
            )
        } else if self.settings.verbose_logging {
            "Verbose logging on. The assistant service picks it up when the app next starts."
                .to_string()
        } else {
            "Verbose logging off".to_string()
        };
        self.set_notice(notice, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::report_url;
    use std::path::Path;

    #[test]
    fn report_links_carry_the_version_and_where_the_files_are() {
        let url = report_url(
            "md-wingman 0.1.0 on macos 15.1 (aarch64)",
            Some(Path::new(
                "/Users/me/Library/Logs/md-wingman/md-wingman.log",
            )),
            Some(Path::new(
                "/Users/me/Library/Logs/md-wingman/crashes/crash-1.txt",
            )),
        );
        assert!(url.starts_with(
            "https://github.com/deanberris/md-wingman/issues/new?title=Crash%3A%20&body="
        ));
        assert!(
            url.contains("md-wingman%200.1.0%20on%20macos%2015.1"),
            "{url}"
        );
        assert!(url.contains("crash-1.txt"), "{url}");
        assert!(!url.contains(' ') && !url.contains('\n'), "{url}");
        let plain = report_url("md-wingman 0.1.0", None, None);
        assert!(plain.contains("?title=&body="), "{plain}");
    }
}
