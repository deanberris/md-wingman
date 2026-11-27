//! When to look for updates and how to show them, without getting in the way
//! of writing.
//!
//! A copy built by the release workflow checks GitHub 30 seconds after
//! launch and once a day after that, downloads and checks a newer version in
//! the background, and installs it when the app quits. The only sign is a
//! note at the right of the status bar; clicking it (or md-wingman ▸ Restart
//! to Update) asks about unsaved changes as quitting does, then restarts into
//! the new version. A copy built from source only notes that a new version
//! is out. Nothing opens a dialog, and failures of the automatic checks go to
//! the log, not the screen. md-wingman ▸ Check for Updates… reports what it
//! finds in the status bar.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use semver::Version;

use crate::update::{Available, Download, Installation, UpdateSource, newest};
use crate::workspace::Workspace;
use crate::{CheckForUpdates, RestartToUpdate, ToggleAutoUpdate};

/// After launch, so the first check doesn't compete with starting up.
pub const FIRST_CHECK: Duration = Duration::from_secs(30);
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Checking,
    Downloading(Version),
    /// Downloaded and checked; installs when the app quits.
    Ready(Version),
    /// A newer version is out that this copy can't install itself, and
    /// what to do instead.
    Available {
        update: Available,
        how: String,
    },
}

pub struct Updates {
    source: Arc<dyn UpdateSource>,
    pub(crate) installation: Installation,
    pub(crate) state: UpdateState,
    timer: Option<Task<()>>,
    work: Option<Task<()>>,
    /// Someone asked for this check, so say what it found, even "up to
    /// date". Automatic checks stay quiet.
    asked: bool,
}

impl Updates {
    pub fn new(source: Arc<dyn UpdateSource>) -> Self {
        let installation = source.installation();
        log::info!(target: "update", "this copy: {installation:?}");
        Updates {
            source,
            installation,
            state: UpdateState::Idle,
            timer: None,
            work: None,
            asked: false,
        }
    }
}

/// An update that installs when the app quits.
struct PendingInstall {
    source: Arc<dyn UpdateSource>,
    staged: PathBuf,
    bundle: PathBuf,
}

#[derive(Default)]
struct Pending {
    install: Option<PendingInstall>,
    /// Open the app again afterwards: the user chose Restart to Update.
    relaunch: bool,
}

impl Global for Pending {}

/// Installs a staged update whenever the app quits, however it's asked to.
pub fn init(cx: &mut App) {
    cx.on_app_quit(|cx| {
        install_on_quit(cx);
        async {}
    })
    .detach();
}

/// Starts installing the staged update, if there is one. Runs as the app
/// quits; the installer waits for it to exit.
pub fn install_on_quit(cx: &mut App) {
    let pending = cx.default_global::<Pending>();
    let Some(install) = pending.install.take() else {
        return;
    };
    let relaunch = pending.relaunch;
    if let Err(e) = install
        .source
        .install(&install.staged, &install.bundle, relaunch)
    {
        log::error!(target: "update", "couldn't install the update: {e}");
    }
}

/// The user cancelled quitting, so a later quit shouldn't restart the app.
pub fn cancel_restart(cx: &mut App) {
    cx.default_global::<Pending>().relaunch = false;
}

/// Whether quitting now restarts into the update, for tests.
#[cfg(test)]
pub fn restarting(cx: &App) -> bool {
    cx.try_global::<Pending>().is_some_and(|p| p.relaunch)
}

impl Workspace {
    /// Schedules the first check, if this copy checks at all.
    pub(crate) fn start_update_checks(&mut self, cx: &mut Context<Self>) {
        if !self.updates.installation.checks() {
            return;
        }
        self.schedule_update_check(FIRST_CHECK, cx);
    }

    fn schedule_update_check(&mut self, after: Duration, cx: &mut Context<Self>) {
        self.updates.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let _ = this.update(cx, |this, cx| {
                let every = this.updates.source.check_every();
                this.schedule_update_check(every, cx);
                if this.settings.check_for_updates {
                    this.check_for_updates(false, cx);
                }
            });
        }));
    }

    pub(crate) fn on_check_for_updates(
        &mut self,
        _: &CheckForUpdates,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.updates.installation.checks() {
            self.set_notice(
                "This copy of md-wingman isn't an installed app, so it doesn't check for updates",
                cx,
            );
            return;
        }
        self.check_for_updates(true, cx);
    }

    pub(crate) fn on_toggle_auto_update(
        &mut self,
        _: &ToggleAutoUpdate,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on = !self.settings.check_for_updates;
        self.settings.check_for_updates = on;
        self.save_settings(cx);
        log::info!(target: "update", "automatic checks {}", if on { "on" } else { "off" });
        if !on {
            self.set_notice("Stopped checking for updates automatically", cx);
            return;
        }
        self.set_notice("md-wingman checks for updates once a day", cx);
        if self.updates.installation.checks() && self.updates.state == UpdateState::Idle {
            self.check_for_updates(false, cx);
        }
    }

    pub(crate) fn on_restart_to_update(
        &mut self,
        _: &RestartToUpdate,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restart_to_update(cx);
    }

    fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        match &self.updates.state {
            UpdateState::Ready(version) => {
                log::info!(target: "update", "restarting to install {version}");
                cx.default_global::<Pending>().relaunch = true;
                crate::quit::request_quit(cx);
            }
            UpdateState::Available { how, .. } => {
                let how = how.clone();
                self.set_notice(how, cx);
            }
            UpdateState::Downloading(version) => {
                let note = format!("Still downloading md-wingman {version}");
                self.set_notice(note, cx);
            }
            UpdateState::Idle | UpdateState::Checking => self.set_notice(
                "No update is ready to install. md-wingman ▸ Check for Updates… looks for one",
                cx,
            ),
        }
    }

    fn check_for_updates(&mut self, asked: bool, cx: &mut Context<Self>) {
        let busy = match &self.updates.state {
            UpdateState::Checking => Some("Already checking for updates…".to_string()),
            UpdateState::Downloading(v) => Some(format!("Downloading md-wingman {v}…")),
            UpdateState::Ready(v) => Some(format!(
                "md-wingman {v} is ready: it installs when you quit, or now with md-wingman ▸ Restart to Update"
            )),
            UpdateState::Idle | UpdateState::Available { .. } => None,
        };
        if let Some(note) = busy {
            if asked {
                self.set_notice(note, cx);
            }
            return;
        }
        self.updates.asked = asked;
        self.updates.state = UpdateState::Checking;
        if asked {
            self.set_notice("Checking for updates…", cx);
        }
        let source = self.updates.source.clone();
        let prereleases = self.settings.update_prereleases;
        let task = cx.background_executor().spawn(async move {
            let current = source.current();
            source
                .releases()
                .map(|releases| newest(&releases, &current, prereleases))
        });
        self.updates.work = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| this.update_checked(result, cx));
        }));
    }

    fn update_checked(
        &mut self,
        result: Result<Option<Available>, String>,
        cx: &mut Context<Self>,
    ) {
        let asked = self.updates.asked;
        self.updates.state = UpdateState::Idle;
        let update = match result {
            Ok(Some(update)) => update,
            Ok(None) => {
                let current = self.updates.source.current();
                log::info!(target: "update", "{current} is the latest version");
                if asked {
                    self.set_notice(format!("md-wingman {current} is the latest version"), cx);
                }
                return;
            }
            Err(e) => {
                log::warn!(target: "update", "couldn't check for updates: {e}");
                if asked {
                    self.set_notice(format!("Couldn't check for updates: {e}"), cx);
                }
                return;
            }
        };
        log::info!(target: "update", "md-wingman {} is out", update.version);
        if let Installation::Release { bundle } = &self.updates.installation
            && let Some(download) = &update.download
        {
            let (bundle, download) = (bundle.clone(), download.clone());
            self.download_update(bundle, download, update.version, cx);
            return;
        }
        let how = manual_update(&self.updates.installation, &update);
        if asked {
            self.set_notice(how.clone(), cx);
        }
        self.updates.state = UpdateState::Available { update, how };
        cx.notify();
    }

    fn download_update(
        &mut self,
        bundle: PathBuf,
        download: Download,
        version: Version,
        cx: &mut Context<Self>,
    ) {
        if self.updates.asked {
            self.set_notice(format!("Downloading md-wingman {version}…"), cx);
        }
        self.updates.state = UpdateState::Downloading(version.clone());
        let source = self.updates.source.clone();
        let task = cx.background_executor().spawn({
            let version = version.clone();
            async move { source.stage(&download, &version) }
        });
        self.updates.work = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.update_staged(result, bundle, version, cx)
            });
        }));
        cx.notify();
    }

    fn update_staged(
        &mut self,
        result: Result<PathBuf, String>,
        bundle: PathBuf,
        version: Version,
        cx: &mut Context<Self>,
    ) {
        let asked = self.updates.asked;
        let staged = match result {
            Ok(staged) => staged,
            Err(e) => {
                log::error!(target: "update", "couldn't get md-wingman {version} ready: {e}");
                self.updates.state = UpdateState::Idle;
                if asked {
                    self.set_notice(format!("Couldn't download md-wingman {version}: {e}"), cx);
                }
                cx.notify();
                return;
            }
        };
        cx.default_global::<Pending>().install = Some(PendingInstall {
            source: self.updates.source.clone(),
            staged,
            bundle,
        });
        if asked {
            self.set_notice(
                format!("md-wingman {version} is ready: it installs when you quit"),
                cx,
            );
        }
        self.updates.state = UpdateState::Ready(version);
        cx.notify();
    }

    /// The note at the right of the status bar, when there's an update.
    pub(crate) fn render_update_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let link = cx.theme().link;
        let note = |id: &'static str, label: String| {
            div()
                .id(id)
                .test_support()
                .text_color(link)
                .cursor_pointer()
                .child(label)
        };
        match &self.updates.state {
            UpdateState::Ready(version) => Some(
                note("update-ready", format!("↑ {version} ready · restart"))
                    .on_click(cx.listener(|this, _, _, cx| this.restart_to_update(cx)))
                    .into_any_element(),
            ),
            UpdateState::Available { update, .. } => {
                let page = update.page.clone();
                Some(
                    note("update-available", format!("↑ {} is out", update.version))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let UpdateState::Available { how, .. } = &this.updates.state {
                                let how = how.clone();
                                this.set_notice(how, cx);
                            }
                            crate::about::open_link(&page, cx);
                        }))
                        .into_any_element(),
                )
            }
            _ => None,
        }
    }
}

/// What to do about an update this copy can't install itself.
fn manual_update(installation: &Installation, update: &Available) -> String {
    let version = &update.version;
    match installation {
        Installation::Source => format!(
            "md-wingman {version} is out. You built this copy from source: git pull, then run scripts/install.sh"
        ),
        Installation::Stuck { reason } => format!("md-wingman {version} is out. {reason}"),
        Installation::Release { .. } | Installation::Unbundled => format!(
            "md-wingman {version} is out, without a download for this Mac. The release page has the details"
        ),
    }
}
