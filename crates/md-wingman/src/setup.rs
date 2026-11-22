//! The setup wizard: what md-wingman needs before the assistant can help,
//! with the commands to do it.
//!
//! 1. Claude Code, installed and signed in (`claude auth status`).
//! 2. Optionally, a TypeSafe API key for the Jev pre-filter, in the Keychain.
//!
//! It opens in the Assistant pane at first launch and from Assistant ▸
//! Setup…. It never takes focus from the editor and never opens by itself
//! while you write. Each command can be copied, and on macOS run in a
//! Terminal window. While Claude Code isn't ready, it checks again every few
//! seconds in the background, so signing in in Terminal shows up here.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use gpui_kit::component::Theme;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use wingman_client::AuthMode;

use crate::ShowSetup;
use crate::assistant::auth_mode_from_env;
use crate::workspace::Workspace;

/// Claude Code's own installer.
pub const INSTALL_COMMAND: &str = "curl -fsSL https://claude.ai/install.sh | bash";
/// Signs Claude Code in with a Claude plan.
pub const LOGIN_COMMAND: &str = "claude auth login";
/// Saves the TypeSafe key where md-wingman looks for it. `-w` last, with no
/// value, makes `security` ask for the key rather than take it on the
/// command line, where it would land in the shell's history.
pub const JEV_KEY_COMMAND: &str =
    "security add-generic-password -U -s md-wingman -a typesafe-api-key -w";

/// How often to look again while Claude Code isn't ready.
pub const RECHECK_EVERY: Duration = Duration::from_secs(3);
/// How long `claude auth status` may take.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// Where Claude Code stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeStatus {
    NotInstalled,
    SignedOut {
        path: PathBuf,
    },
    SignedIn {
        path: PathBuf,
        method: Option<String>,
    },
    /// Installed, but `claude auth status` didn't say.
    Unknown {
        path: PathBuf,
        error: String,
    },
}

impl ClaudeStatus {
    pub fn ready(&self) -> bool {
        matches!(self, ClaudeStatus::SignedIn { .. })
    }
}

/// Looks at Claude Code and runs commands for the wizard: [`SystemProbe`]
/// for real, a fake in tests.
pub trait SetupProbe: Send + Sync {
    /// Blocks, so call it off the main thread.
    fn check_claude(&self) -> ClaudeStatus;
    /// Whether commands can be run in a Terminal window here.
    fn can_run_in_terminal(&self) -> bool;
    /// Opens a Terminal window running `command`.
    fn run_in_terminal(&self, command: &str) -> Result<(), String>;
}

pub struct SystemProbe;

impl SetupProbe for SystemProbe {
    fn check_claude(&self) -> ClaudeStatus {
        let Some(path) = find_claude() else {
            return ClaudeStatus::NotInstalled;
        };
        match auth_status(&path) {
            Ok(output) => parse_auth_status(path, &output),
            Err(error) => ClaudeStatus::Unknown { path, error },
        }
    }

    fn can_run_in_terminal(&self) -> bool {
        cfg!(target_os = "macos")
    }

    fn run_in_terminal(&self, command: &str) -> Result<(), String> {
        // A `.command` file opens in Terminal and runs, with no need to ask
        // for permission to control Terminal.
        let dir = std::env::temp_dir().join("md-wingman");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let script = dir.join(format!("setup-{}.command", std::process::id()));
        std::fs::write(&script, terminal_script(command)).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let status = Command::new("open")
            .args(["-a", "Terminal"])
            .arg(&script)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("open exited with {status}"))
        }
    }
}

/// The script a Terminal window runs: a login shell, so PATH includes what
/// the installer added.
pub fn terminal_script(command: &str) -> String {
    format!(
        "#!/bin/zsh -l\n\
         echo '$ {}'\n\
         {command}\n\
         echo\n\
         echo 'Done. You can close this window and go back to md-wingman.'\n",
        command.replace('\'', r"'\''")
    )
}

/// Finds the `claude` executable the way the service does: WINGMAN_CLAUDE_PATH,
/// then PATH, then the usual install locations (an app started from Finder
/// doesn't get the shell's PATH).
fn find_claude() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WINGMAN_CLAUDE_PATH") {
        return Some(PathBuf::from(path));
    }
    claude_candidates(
        std::env::var_os("PATH").as_deref(),
        dirs::home_dir().as_deref(),
    )
    .into_iter()
    .find(|p| is_executable(p))
}

pub fn claude_candidates(path: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = path
        .map(|p| std::env::split_paths(p).map(|d| d.join("claude")).collect())
        .unwrap_or_default();
    if let Some(home) = home {
        out.push(home.join(".local/bin/claude"));
        out.push(home.join(".claude/local/claude"));
    }
    out.push("/opt/homebrew/bin/claude".into());
    out.push("/usr/local/bin/claude".into());
    out
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Runs `claude auth status`, giving up after a few seconds. Signed out, it
/// may exit with an error and still print its JSON, so only the output counts.
fn auth_status(claude: &Path) -> Result<String, String> {
    let mut child = Command::new(claude)
        .args(["auth", "status", "--json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run {}: {e}", claude.display()))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > STATUS_TIMEOUT => {
                let _ = child.kill();
                return Err("claude auth status didn't answer".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if stdout.trim().is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().to_string());
    }
    Ok(stdout)
}

/// Reads `claude auth status --json`.
pub fn parse_auth_status(path: PathBuf, output: &str) -> ClaudeStatus {
    let value: serde_json::Value = match serde_json::from_str(output.trim()) {
        Ok(v) => v,
        Err(_) => {
            return ClaudeStatus::Unknown {
                path,
                error: output.trim().lines().next().unwrap_or_default().to_string(),
            };
        }
    };
    match value.get("loggedIn").and_then(|v| v.as_bool()) {
        Some(true) => ClaudeStatus::SignedIn {
            path,
            method: value
                .get("authMethod")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        },
        Some(false) => ClaudeStatus::SignedOut { path },
        None => ClaudeStatus::Unknown {
            path,
            error: "claude auth status didn't say whether you're signed in".into(),
        },
    }
}

/// The wizard, while it's open.
#[derive(Default)]
pub struct SetupWizard {
    /// `None` until the first check comes back.
    pub claude: Option<ClaudeStatus>,
    pub checking: bool,
    /// Skipped the Jev step.
    pub jev_skipped: bool,
    /// Bumped on every check, so only the latest one counts and only one
    /// recheck is ever waiting.
    generation: u64,
}

/// Puts a path in a command, quoted for the shell when it needs to be.
fn shell_path(path: &Path) -> String {
    let s = path.display().to_string();
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        s
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

impl Workspace {
    pub(crate) fn on_show_setup(&mut self, _: &ShowSetup, _: &mut Window, cx: &mut Context<Self>) {
        self.open_setup(cx);
    }

    /// Opens the wizard and looks at Claude Code. Doesn't take focus, so
    /// whatever you're typing keeps going to the editor.
    pub(crate) fn open_setup(&mut self, cx: &mut Context<Self>) {
        if self.setup.is_none() {
            self.setup = Some(SetupWizard::default());
        }
        self.setup_check(cx);
        cx.notify();
    }

    pub(crate) fn setup_check(&mut self, cx: &mut Context<Self>) {
        let Some(wizard) = self.setup.as_mut() else {
            return;
        };
        wizard.generation += 1;
        wizard.checking = true;
        let generation = wizard.generation;
        let probe = self.probe.clone();
        let check = cx
            .background_executor()
            .spawn(async move { probe.check_claude() });
        cx.spawn(async move |this, cx| {
            let status = check.await;
            let _ = this.update(cx, |this, cx| this.setup_checked(generation, status, cx));
        })
        .detach();
        cx.notify();
    }

    fn setup_checked(&mut self, generation: u64, status: ClaudeStatus, cx: &mut Context<Self>) {
        let Some(wizard) = self.setup.as_mut() else {
            return;
        };
        if wizard.generation != generation {
            return;
        }
        wizard.checking = false;
        let ready = status.ready();
        wizard.claude = Some(status);
        if !ready {
            let timer = cx.background_executor().timer(RECHECK_EVERY);
            cx.spawn(async move |this, cx| {
                timer.await;
                let _ = this.update(cx, |this, cx| {
                    if this
                        .setup
                        .as_ref()
                        .is_some_and(|w| w.generation == generation)
                    {
                        this.setup_check(cx);
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// Closes the wizard. It comes back at the next launch only if Claude
    /// Code still wasn't ready.
    fn close_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ready = self
            .setup
            .as_ref()
            .and_then(|w| w.claude.as_ref())
            .is_some_and(ClaudeStatus::ready);
        self.setup = None;
        if ready && !self.settings.setup_done {
            self.settings.setup_done = true;
            self.save_settings(cx);
        }
        self.editor()
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn copy_command(&mut self, command: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(command));
        self.set_notice("Copied. Paste it into Terminal.", cx);
    }

    fn run_command(&mut self, command: String, cx: &mut Context<Self>) {
        match self.probe.run_in_terminal(&command) {
            Ok(()) => self.set_notice(
                "Opened Terminal. md-wingman will notice when it's done.",
                cx,
            ),
            Err(e) => self.set_notice(format!("Couldn't open Terminal: {e}"), cx),
        }
    }

    fn jev_ready(&self) -> bool {
        self.settings.jev_prefilter && matches!(self.continuous.cached_key(), Some(Some(_)))
    }

    pub(crate) fn render_setup(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let wizard = self.setup.as_ref()?;
        let can_run = self.probe.can_run_in_terminal();
        let api_key = auth_mode_from_env() == AuthMode::ApiKey;

        let heading = |text: &str| {
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(text.to_string())
        };
        let note = |text: String| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let mark = |done: bool| {
            div()
                .flex_none()
                .w(px(16.))
                .font_family(theme.mono_font_family.clone())
                .text_color(if done {
                    theme.success
                } else {
                    theme.muted_foreground
                })
                .child(if done { "✓" } else { "○" })
        };
        // A command to copy, and to run in Terminal where that works.
        let command_row =
            |id: &'static str, shown: &str, run: Option<String>, cx: &mut Context<Self>| {
                let copy = shown.to_string();
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.muted)
                            .text_xs()
                            .font_family(theme.mono_font_family.clone())
                            .child(shown.to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!("setup-copy-{id}")))
                                    .label("Copy")
                                    .xsmall()
                                    .outline()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy_command(copy.clone(), cx)
                                    })),
                            )
                            .when_some(run.filter(|_| can_run), |row, run| {
                                row.child(
                                    Button::new(SharedString::from(format!("setup-run-{id}")))
                                        .label("Run in Terminal")
                                        .xsmall()
                                        .primary()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.run_command(run.clone(), cx)
                                        })),
                                )
                            }),
                    )
            };

        // Step 1: Claude Code.
        let claude_ready = api_key || wizard.claude.as_ref().is_some_and(ClaudeStatus::ready);
        let mut claude = v_flex().gap_2();
        if api_key {
            claude = claude.child(note(
                "Using an API key (WINGMAN_AUTH=api_key), so Claude Code doesn't need to be signed in."
                    .into(),
            ));
        } else {
            match &wizard.claude {
                None => claude = claude.child(note("Looking for Claude Code…".into())),
                Some(ClaudeStatus::NotInstalled) => {
                    claude = claude
                        .child(note(
                            "Claude Code isn't installed. md-wingman uses it to reach Claude with \
                             your Claude plan. Install it, then sign in:"
                                .into(),
                        ))
                        .child(command_row(
                            "install",
                            INSTALL_COMMAND,
                            Some(format!(
                                "{INSTALL_COMMAND} && ~/.local/bin/claude auth login"
                            )),
                            cx,
                        ))
                        .child(command_row("login", LOGIN_COMMAND, None, cx));
                }
                Some(ClaudeStatus::SignedOut { path }) => {
                    claude = claude
                        .child(note(
                            "Claude Code is installed but not signed in. Sign in with your Claude \
                             account; a browser window opens to finish."
                                .into(),
                        ))
                        .child(command_row(
                            "login",
                            LOGIN_COMMAND,
                            Some(format!("{} auth login", shell_path(path))),
                            cx,
                        ));
                }
                Some(ClaudeStatus::Unknown { path, error }) => {
                    claude = claude
                        .child(note(format!(
                            "Couldn't tell whether Claude Code is signed in ({error}). If it isn't, sign in:"
                        )))
                        .child(command_row(
                            "login",
                            LOGIN_COMMAND,
                            Some(format!("{} auth login", shell_path(path))),
                            cx,
                        ));
                }
                Some(ClaudeStatus::SignedIn { method, .. }) => {
                    claude = claude.child(note(match method.as_deref() {
                        Some(m) => format!("Claude Code is signed in ({m})."),
                        None => "Claude Code is signed in.".into(),
                    }));
                }
            }
            if !claude_ready && wizard.claude.is_some() {
                claude = claude.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Button::new("setup-recheck")
                                .label("Check again")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| this.setup_check(cx))),
                        )
                        .child(note(if wizard.checking {
                            "Checking…".into()
                        } else {
                            "Checks again by itself every few seconds.".into()
                        })),
                );
            }
        }

        // Step 2: Jev, optional.
        let jev_ready = self.jev_ready();
        let mut jev = v_flex().gap_2();
        if jev_ready {
            jev = jev.child(note("Jev pre-filter on, with your TypeSafe key.".into()));
        } else if wizard.jev_skipped {
            jev = jev.child(note(
                "Skipped. Turn it on any time from Assistant ▸ Jev Pre-filter.".into(),
            ));
        } else {
            jev = jev
                .child(note(
                    "Optional. While checking as you write, Jev (TypeSafe's grading model) \
                     grades each paragraph first, so Claude only looks at the ones that need it. \
                     It needs a TypeSafe API key from console.typesafe.ai/keys, kept in your \
                     macOS Keychain."
                        .into(),
                ))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("setup-jev-use")
                                .label("Add key and turn on Jev")
                                .xsmall()
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_jev_prefilter(true, window, cx)
                                })),
                        )
                        .child(
                            Button::new("setup-jev-skip")
                                .label("Skip")
                                .xsmall()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(w) = this.setup.as_mut() {
                                        w.jev_skipped = true;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(note(
                    "Or save the key from Terminal (it asks for the key, so it stays out of your \
                     shell history), then turn on Assistant ▸ Jev Pre-filter:"
                        .into(),
                ))
                .child(command_row("jev", JEV_KEY_COMMAND, None, cx));
        }

        let step = |done: bool, title: &str, body: Div| {
            h_flex().gap_2().items_start().child(mark(done)).child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(heading(title))
                    .child(body),
            )
        };

        Some(
            v_flex()
                .id("setup")
                .gap_3()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Set up md-wingman"),
                )
                .child(step(claude_ready, "1. Claude Code", claude))
                .child(step(
                    jev_ready || wizard.jev_skipped,
                    "2. Jev pre-filter (optional)",
                    jev,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Button::new("setup-done")
                                .label(if claude_ready { "Done" } else { "Finish later" })
                                .xsmall()
                                .when(claude_ready, |b| b.primary())
                                .when(!claude_ready, |b| b.ghost())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.close_setup(window, cx)),
                                ),
                        )
                        .when(!claude_ready, |row| {
                            row.child(note(
                                "This comes back next time until Claude Code is signed in. \
                             Reopen it from Assistant ▸ Setup…."
                                    .into(),
                            ))
                        }),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's glob would shadow `#[test]`.
    use super::{ClaudeStatus, claude_candidates, parse_auth_status, shell_path, terminal_script};
    use std::path::{Path, PathBuf};

    #[test]
    fn reads_claude_auth_status() {
        let path = PathBuf::from("/bin/claude");
        let signed_in =
            r#"{"loggedIn": true, "authMethod": "claude.ai", "apiProvider": "firstParty"}"#;
        assert_eq!(
            parse_auth_status(path.clone(), signed_in),
            ClaudeStatus::SignedIn {
                path: path.clone(),
                method: Some("claude.ai".into())
            }
        );
        assert_eq!(
            parse_auth_status(path.clone(), r#"{"loggedIn": false}"#),
            ClaudeStatus::SignedOut { path: path.clone() }
        );
        assert!(matches!(
            parse_auth_status(path.clone(), "error: unknown command"),
            ClaudeStatus::Unknown { .. }
        ));
        assert!(matches!(
            parse_auth_status(path, "{}"),
            ClaudeStatus::Unknown { .. }
        ));
    }

    #[test]
    fn looks_where_the_service_looks() {
        let path = std::env::join_paths(["/a", "/b"]).unwrap();
        let found = claude_candidates(Some(&path), Some(Path::new("/Users/me")));
        let found: Vec<_> = found.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(
            found,
            [
                "/a/claude",
                "/b/claude",
                "/Users/me/.local/bin/claude",
                "/Users/me/.claude/local/claude",
                "/opt/homebrew/bin/claude",
                "/usr/local/bin/claude",
            ]
        );
    }

    #[test]
    fn quotes_paths_with_spaces() {
        assert_eq!(shell_path(Path::new("/usr/bin/claude")), "/usr/bin/claude");
        assert_eq!(
            shell_path(Path::new("/Users/me/My Apps/claude")),
            "'/Users/me/My Apps/claude'"
        );
    }

    #[test]
    fn terminal_script_echoes_then_runs() {
        let script = terminal_script("claude auth login");
        assert!(script.starts_with("#!/bin/zsh -l\n"));
        assert!(script.contains("echo '$ claude auth login'\nclaude auth login\n"));
    }
}
