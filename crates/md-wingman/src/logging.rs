//! Setting up the log file, and the Help menu's Show Log File and Verbose
//! Logging. See the `wingman-log` crate for where the file is and what is
//! kept out of it.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::*;
use wingman_log::LevelFilter;

use crate::settings::Settings;
use crate::workspace::Workspace;
use crate::{ShowLogFile, ToggleVerboseLogging};

/// `WINGMAN_LOG` was set, so it decides the level, not the settings.
static LEVEL_FROM_ENV: AtomicBool = AtomicBool::new(false);

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
    let config = wingman_log::Config::from_env();
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
        let opened = if cfg!(target_os = "macos") {
            std::process::Command::new("open")
                .arg("-R")
                .arg(path)
                .spawn()
        } else {
            std::process::Command::new("xdg-open")
                .arg(path.parent().unwrap_or(path))
                .spawn()
        };
        match opened {
            Ok(_) => self.set_notice(format!("Log file: {}", path.display()), cx),
            Err(e) => self.set_notice(format!("Log file: {} ({e})", path.display()), cx),
        }
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
