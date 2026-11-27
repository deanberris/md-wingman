//! Updates: checked in the background, shown only as a note in the status
//! bar, installed when the app quits or on Restart to Update.

use std::path::{Path, PathBuf};

use gpui_kit::TestAppContext;
use semver::Version;

use super::*;
use crate::update::{Asset, Download, Installation, Release, UpdateSource, asset_name};
use crate::updater::{FIRST_CHECK, UpdateState};

/// GitHub and the app bundle, scripted.
pub struct FakeUpdates {
    installation: Installation,
    releases: Mutex<Result<Vec<Release>, String>>,
    stage_error: Mutex<Option<String>>,
    checks: AtomicUsize,
    staged: Mutex<Vec<Version>>,
    /// Each install: staged app, bundle, relaunch.
    installs: Mutex<Vec<(PathBuf, PathBuf, bool)>>,
}

impl FakeUpdates {
    pub fn new(installation: Installation) -> Arc<Self> {
        Arc::new(FakeUpdates {
            installation,
            releases: Mutex::new(Ok(Vec::new())),
            stage_error: Mutex::new(None),
            checks: AtomicUsize::new(0),
            staged: Mutex::default(),
            installs: Mutex::default(),
        })
    }

    fn publish(&self, tag: &str) {
        let version = Version::parse(tag.trim_start_matches('v')).unwrap();
        let asset = |name: String| Asset {
            browser_download_url: format!("https://example.invalid/{name}"),
            name,
        };
        let release = Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: true,
            html_url: format!("https://github.com/deanberris/md-wingman/releases/tag/{tag}"),
            assets: vec![asset(asset_name(&version)), asset("SHA256SUMS.txt".into())],
        };
        self.releases
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .push(release);
    }

    fn fail_checks(&self, error: &str) {
        *self.releases.lock().unwrap() = Err(error.into());
    }

    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }

    fn staged(&self) -> Vec<Version> {
        self.staged.lock().unwrap().clone()
    }

    fn installs(&self) -> Vec<(PathBuf, PathBuf, bool)> {
        self.installs.lock().unwrap().clone()
    }
}

impl UpdateSource for FakeUpdates {
    fn current(&self) -> Version {
        Version::new(0, 1, 0)
    }
    fn installation(&self) -> Installation {
        self.installation.clone()
    }
    fn releases(&self) -> Result<Vec<Release>, String> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.releases.lock().unwrap().clone()
    }
    fn stage(&self, _: &Download, version: &Version) -> Result<PathBuf, String> {
        if let Some(e) = self.stage_error.lock().unwrap().clone() {
            return Err(e);
        }
        self.staged.lock().unwrap().push(version.clone());
        Ok(PathBuf::from(format!("/cache/{version}/md-wingman.app")))
    }
    /// A day of the test clock runs every timer in the workspace many times
    /// over, so "tomorrow" is a minute away here.
    fn check_every(&self) -> std::time::Duration {
        std::time::Duration::from_secs(60)
    }
    fn install(&self, staged: &Path, bundle: &Path, relaunch: bool) -> Result<(), String> {
        self.installs
            .lock()
            .unwrap()
            .push((staged.into(), bundle.into(), relaunch));
        Ok(())
    }
}

const BUNDLE: &str = "/Applications/md-wingman.app";

fn release_build() -> Setup {
    Setup {
        installation: Installation::Release {
            bundle: BUNDLE.into(),
        },
        ..Setup::default()
    }
}

fn state(h: &Harness, cx: &mut TestAppContext) -> UpdateState {
    h.read(cx, |ws, _| ws.updates.state.clone())
}

fn dispatch(h: &Harness, cx: &mut TestAppContext, action: impl gpui_kit::Action) {
    h.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(action), cx)
    });
    cx.run_until_parked();
    h.frame(cx);
}

fn wait_for_first_check(h: &Harness, cx: &mut TestAppContext) {
    h.wait(cx, FIRST_CHECK.as_millis() as u64);
}

#[gpui_kit::test]
fn a_release_build_downloads_quietly_and_installs_on_quit(cx: &mut TestAppContext) {
    let h = open(cx, release_build());
    h.updates.publish("v0.2.0");
    // Nothing at launch: the first check waits so it doesn't slow startup.
    assert_eq!(h.updates.checks(), 0);

    wait_for_first_check(&h, cx);
    assert_eq!(h.updates.checks(), 1);
    assert_eq!(h.updates.staged(), [Version::new(0, 2, 0)]);
    assert_eq!(state(&h, cx), UpdateState::Ready(Version::new(0, 2, 0)));
    // Only the note in the status bar: no notice, no prompt.
    assert!(h.exists(cx, "update-ready"));
    assert_eq!(h.notice(cx), "");
    assert!(!cx.has_pending_prompt());

    // Quitting installs it, without opening it again.
    h.press(cx, &shortcut("q"));
    cx.update(crate::updater::install_on_quit);
    assert_eq!(
        h.updates.installs(),
        [(
            PathBuf::from("/cache/0.2.0/md-wingman.app"),
            PathBuf::from(BUNDLE),
            false
        )]
    );
}

#[gpui_kit::test]
fn restart_to_update_asks_about_unsaved_changes_then_relaunches(cx: &mut TestAppContext) {
    let h = open(cx, release_build());
    h.updates.publish("v0.2.0");
    wait_for_first_check(&h, cx);
    h.type_text(cx, "Unsaved words.");

    h.click(cx, "update-ready");
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!cx.update(|cx| crate::quit::quit_requested(cx)));
    // Cancelling means a later plain quit doesn't restart the app.
    assert!(!cx.update(|cx| crate::updater::restarting(cx)));

    dispatch(&h, cx, crate::RestartToUpdate);
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(cx.update(|cx| crate::quit::quit_requested(cx)));
    cx.update(crate::updater::install_on_quit);
    let installs = h.updates.installs();
    assert_eq!(installs.len(), 1);
    assert!(installs[0].2, "relaunches after installing");
}

#[gpui_kit::test]
fn a_source_build_only_says_a_new_version_is_out(cx: &mut TestAppContext) {
    let h = open(
        cx,
        Setup {
            installation: Installation::Source,
            ..Setup::default()
        },
    );
    h.updates.publish("v0.2.0");
    wait_for_first_check(&h, cx);
    assert!(h.updates.staged().is_empty());
    assert!(h.exists(cx, "update-available"));
    assert_eq!(h.notice(cx), "");

    h.click(cx, "update-available");
    assert!(h.notice(cx).contains("scripts/install.sh"));
    let links = cx.update(|cx| {
        cx.try_global::<crate::about::OpenedLinks>()
            .map(|l| l.0.clone())
            .unwrap_or_default()
    });
    assert_eq!(
        links,
        ["https://github.com/deanberris/md-wingman/releases/tag/v0.2.0"]
    );
    cx.update(crate::updater::install_on_quit);
    assert!(h.updates.installs().is_empty());
}

#[gpui_kit::test]
fn check_for_updates_says_what_it_found(cx: &mut TestAppContext) {
    let h = open(cx, release_build());
    dispatch(&h, cx, crate::CheckForUpdates);
    assert_eq!(h.notice(cx), "md-wingman 0.1.0 is the latest version");

    h.updates.fail_checks("couldn't reach GitHub: offline");
    dispatch(&h, cx, crate::CheckForUpdates);
    assert_eq!(
        h.notice(cx),
        "Couldn't check for updates: couldn't reach GitHub: offline"
    );
}

#[gpui_kit::test]
fn automatic_checks_fail_quietly_and_try_again_tomorrow(cx: &mut TestAppContext) {
    let h = open(cx, release_build());
    h.updates.fail_checks("offline");
    wait_for_first_check(&h, cx);
    assert_eq!(h.updates.checks(), 1);
    assert_eq!(h.notice(cx), "");
    assert_eq!(state(&h, cx), UpdateState::Idle);

    h.wait(cx, 60_000);
    assert_eq!(h.updates.checks(), 2);
}

#[gpui_kit::test]
fn a_failed_download_is_dropped_and_logged(cx: &mut TestAppContext) {
    let h = open(cx, release_build());
    h.updates.publish("v0.2.0");
    *h.updates.stage_error.lock().unwrap() = Some("checksum mismatch".into());
    wait_for_first_check(&h, cx);
    assert_eq!(state(&h, cx), UpdateState::Idle);
    assert!(!h.exists(cx, "update-ready"));
    assert_eq!(h.notice(cx), "");
    cx.update(crate::updater::install_on_quit);
    assert!(h.updates.installs().is_empty());
}

#[gpui_kit::test]
fn automatic_checks_can_be_turned_off(cx: &mut TestAppContext) {
    let h = open(cx, release_build());
    h.updates.publish("v0.2.0");
    dispatch(&h, cx, crate::ToggleAutoUpdate);
    assert!(!h.read(cx, |ws, _| ws.settings.check_for_updates));
    wait_for_first_check(&h, cx);
    assert_eq!(h.updates.checks(), 0);

    // Asking still works.
    dispatch(&h, cx, crate::CheckForUpdates);
    assert_eq!(h.updates.checks(), 1);
    assert_eq!(state(&h, cx), UpdateState::Ready(Version::new(0, 2, 0)));
}

#[gpui_kit::test]
fn a_copy_that_isnt_an_app_never_checks(cx: &mut TestAppContext) {
    let h = open(cx, Setup::default());
    h.updates.publish("v0.2.0");
    wait_for_first_check(&h, cx);
    dispatch(&h, cx, crate::CheckForUpdates);
    assert_eq!(h.updates.checks(), 0);
    assert!(h.notice(cx).contains("isn't an installed app"));
}
