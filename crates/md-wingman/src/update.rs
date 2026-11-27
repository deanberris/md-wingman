//! Updates from GitHub Releases: finding a newer version, downloading and
//! checking it, and swapping it in when the app quits.
//!
//! Everything here runs in the background. The workspace side
//! (`updater.rs`) decides when to check and how to show the result: a quiet
//! note in the status bar, never a dialog.
//!
//! Only builds made by the release workflow install updates themselves; it
//! sets `WINGMAN_DISTRIBUTION=github-release` when it compiles the app. A copy
//! built from source with `scripts/install.sh` only says that a new version is
//! out, because swapping in a binary someone else built would undo the point
//! of building it yourself.
//!
//! Before an update is staged, it must match the SHA-256 in the release's
//! `SHA256SUMS.txt`, pass `codesign --verify --strict --deep`, carry this
//! app's bundle identifier and the expected version, and, when this copy is
//! signed by a Developer ID team, be signed by the same team.

use std::ffi::OsStr;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

/// The releases, newest first.
const RELEASES_API: &str =
    "https://api.github.com/repos/deanberris/md-wingman/releases?per_page=30";
/// This app's bundle identifier, as `scripts/package.sh` writes it.
pub const BUNDLE_ID: &str = "io.github.deanberris.md-wingman";
/// No release is anywhere near this big; a download that is, isn't ours.
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

/// A release, as the GitHub API lists it.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// A newer version than this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub version: Version,
    /// The release page, with its notes.
    pub page: String,
    /// The zipped app for this Mac and the checksums for it, when the
    /// release has both. Without them the update can only be downloaded by
    /// hand.
    pub download: Option<Download>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub name: String,
    pub url: String,
    pub sums_url: String,
}

/// The name of the zipped app the release workflow uploads for `version`.
pub fn asset_name(version: &Version) -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    format!("md-wingman-{version}-macos-{arch}.zip")
}

/// The newest release that is newer than `current`, skipping drafts, and
/// GitHub pre-releases unless `prereleases`. Tags look like `v1.2.3`.
pub fn newest(releases: &[Release], current: &Version, prereleases: bool) -> Option<Available> {
    let (version, release) = releases
        .iter()
        .filter(|r| !r.draft && (prereleases || !r.prerelease))
        .filter_map(|r| Some((parse_tag(&r.tag_name)?, r)))
        .filter(|(v, _)| v > current)
        .max_by(|(a, _), (b, _)| a.cmp(b))?;
    let asset = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
    };
    let name = asset_name(&version);
    let download = match (asset(&name), asset("SHA256SUMS.txt")) {
        (Some(url), Some(sums_url)) => Some(Download {
            name,
            url,
            sums_url,
        }),
        _ => None,
    };
    Some(Available {
        version,
        page: release.html_url.clone(),
        download,
    })
}

fn parse_tag(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

/// The checksum for `name` in a `shasum -a 256` listing, in lowercase hex.
pub fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        // `shasum` marks binary mode with a `*` before the name.
        let file = file.trim_start().trim_start_matches('*');
        let valid = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit());
        (file == name && valid).then(|| hash.to_ascii_lowercase())
    })
}

/// The Team ID in `codesign -dv` output, or `None` for an ad-hoc signature.
pub fn team_identifier(codesign_output: &str) -> Option<String> {
    codesign_output
        .lines()
        .find_map(|l| l.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != "not set")
        .map(str::to_string)
}

/// How this copy of the app was built and where it runs from, which decides
/// what it can do about an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installation {
    /// Built by the release workflow, in an app bundle it can replace.
    Release { bundle: PathBuf },
    /// Built by the release workflow, but where it can't replace itself,
    /// with why.
    Stuck { reason: String },
    /// Built from source into an app bundle: say when there's a new version,
    /// but don't install someone else's build.
    Source,
    /// Not an app bundle (`cargo run`, tests): don't check at all.
    Unbundled,
}

impl Installation {
    pub fn checks(&self) -> bool {
        !matches!(self, Installation::Unbundled)
    }
}

/// Where a running executable puts the app: the `.app` around
/// `Contents/MacOS/<exe>`, if it's in one.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

/// Why an app bundle at `bundle` can't replace itself, if it can't. Only
/// looks at the path; [`detect`] also checks the folder is writable.
pub fn stuck_reason(bundle: &Path) -> Option<String> {
    let path = bundle.to_string_lossy();
    if path.contains("/AppTranslocation/") {
        return Some(
            "macOS is running md-wingman from a temporary copy. Move it to Applications to get updates"
                .into(),
        );
    }
    if path.starts_with("/Volumes/") && path.matches('/').count() <= 3 {
        return Some(
            "md-wingman is running from the disk image. Drag it to Applications to get updates"
                .into(),
        );
    }
    None
}

/// Looks at how this copy was built and where it's running from.
pub fn detect() -> Installation {
    let Some(bundle) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .and_then(|exe| bundle_of(&exe))
    else {
        return Installation::Unbundled;
    };
    if option_env!("WINGMAN_DISTRIBUTION") != Some("github-release") {
        return Installation::Source;
    }
    if let Some(reason) = stuck_reason(&bundle) {
        return Installation::Stuck { reason };
    }
    let Some(folder) = bundle.parent() else {
        return Installation::Unbundled;
    };
    if !writable(folder) {
        return Installation::Stuck {
            reason: format!(
                "md-wingman can't update itself in {}, which it can't write to",
                folder.display()
            ),
        };
    }
    Installation::Release { bundle }
}

fn writable(folder: &Path) -> bool {
    let probe = folder.join(format!(".md-wingman-update-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Finds, fetches and installs updates: [`GitHub`] for real, or a fake in
/// tests, so they need neither the network nor a real app bundle.
pub trait UpdateSource: Send + Sync {
    /// The running version.
    fn current(&self) -> Version;
    fn installation(&self) -> Installation;
    /// Blocks, so call it off the main thread.
    fn releases(&self) -> Result<Vec<Release>, String>;
    /// Downloads and checks an update, and returns the unpacked app, ready
    /// to install. Blocks, so call it off the main thread.
    fn stage(&self, download: &Download, version: &Version) -> Result<PathBuf, String>;
    /// Starts replacing `bundle` with `staged` once this process has exited,
    /// then opens it again if `relaunch`. Returns at once.
    fn install(&self, staged: &Path, bundle: &Path, relaunch: bool) -> Result<(), String>;
    /// How long to wait between automatic checks.
    fn check_every(&self) -> Duration {
        crate::updater::CHECK_EVERY
    }
}

/// GitHub Releases and the app bundle this copy runs from.
pub struct GitHub;

impl UpdateSource for GitHub {
    fn current(&self) -> Version {
        Version::parse(crate::about::VERSION).expect("Cargo's version is semver")
    }

    fn installation(&self) -> Installation {
        detect()
    }

    fn releases(&self) -> Result<Vec<Release>, String> {
        log::info!(target: "update", "checking {RELEASES_API}");
        let mut response = agent(Duration::from_secs(30))
            .get(RELEASES_API)
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| format!("couldn't reach GitHub: {e}"))?;
        response
            .body_mut()
            .read_json()
            .map_err(|e| format!("couldn't read GitHub's list of releases: {e}"))
    }

    fn stage(&self, download: &Download, version: &Version) -> Result<PathBuf, String> {
        let dir = updates_dir().ok_or("there's no cache folder to download to")?;
        // Only one update is kept: whatever was staged before is stale now.
        let _ = std::fs::remove_dir_all(&dir);
        let dir = dir.join(version.to_string());
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
        let result = stage_in(&dir, download, version);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        result
    }

    fn install(&self, staged: &Path, bundle: &Path, relaunch: bool) -> Result<(), String> {
        let log = wingman_log::path()
            .and_then(Path::parent)
            .map(|d| d.join("update.log"))
            .unwrap_or_else(|| PathBuf::from("/dev/null"));
        log::info!(
            target: "update",
            "installing {} over {} after quitting{}",
            staged.display(),
            bundle.display(),
            if relaunch { ", then reopening it" } else { "" }
        );
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| format!("couldn't open {}: {e}", log.display()))?;
        let errors = log_file
            .try_clone()
            .map_err(|e| format!("couldn't open {}: {e}", log.display()))?;
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(INSTALL_SCRIPT)
            .arg("md-wingman-update")
            .arg(std::process::id().to_string())
            .arg(staged)
            .arg(bundle)
            .arg(if relaunch { "1" } else { "0" })
            .stdin(std::process::Stdio::null())
            .stdout(log_file)
            .stderr(errors)
            .spawn()
            .map(drop)
            .map_err(|e| format!("couldn't start the installer: {e}"))
    }
}

/// Waits for the app to exit, then swaps the new bundle in beside the old
/// one with two renames, putting the old one back if the second fails.
/// Arguments: pid, staged app, installed app, relaunch (1 or 0).
const INSTALL_SCRIPT: &str = r#"
set -u
pid=$1 staged=$2 target=$3 relaunch=$4
echo "$(date '+%Y-%m-%d %H:%M:%S') installing $staged over $target"
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
new="$target.updating" old="$target.old"
rm -rf "$new" "$old"
if ! ditto "$staged" "$new"; then
    echo "couldn't copy the update"; rm -rf "$new"; exit 1
fi
if ! mv "$target" "$old"; then
    echo "couldn't move the old app aside"; rm -rf "$new"; exit 1
fi
if mv "$new" "$target"; then
    rm -rf "$old" "$(dirname "$(dirname "$staged")")"
    echo "installed"
else
    echo "couldn't move the update into place; keeping the old app"
    mv "$old" "$target"
fi
if [ "$relaunch" = 1 ]; then open "$target"; fi
"#;

/// Where updates are downloaded and unpacked.
fn updates_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("md-wingman").join("updates"))
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(format!("md-wingman/{}", crate::about::VERSION))
        .build()
        .into()
}

fn stage_in(dir: &Path, download: &Download, version: &Version) -> Result<PathBuf, String> {
    let sums = agent(Duration::from_secs(30))
        .get(&download.sums_url)
        .call()
        .map_err(|e| format!("couldn't download the checksums: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("couldn't read the checksums: {e}"))?;
    let expected = checksum_for(&sums, &download.name)
        .ok_or_else(|| format!("the release has no checksum for {}", download.name))?;

    let zip = dir.join(&download.name);
    log::info!(target: "update", "downloading {}", download.url);
    let actual = fetch(&download.url, &zip)?;
    if actual != expected {
        return Err(format!(
            "{} doesn't match its checksum (expected {expected}, got {actual})",
            download.name
        ));
    }

    let unpacked = dir.join("app");
    run(
        "ditto",
        &[os("-x"), os("-k"), zip.as_os_str(), unpacked.as_os_str()],
    )?;
    let _ = std::fs::remove_file(&zip);
    let app = unpacked.join("md-wingman.app");
    check_app(&app, version)?;
    log::info!(target: "update", "staged {version} at {}", app.display());
    Ok(app)
}

/// Downloads `url` to `to`, returning its SHA-256 in lowercase hex.
fn fetch(url: &str, to: &Path) -> Result<String, String> {
    let mut response = agent(Duration::from_secs(15 * 60))
        .get(url)
        .call()
        .map_err(|e| format!("couldn't download the update: {e}"))?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .reader();
    let mut file =
        std::fs::File::create(to).map_err(|e| format!("couldn't write {}: {e}", to.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("the download broke off: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| format!("couldn't write {}: {e}", to.display()))?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// The checks a staged app must pass before it may replace this one.
fn check_app(app: &Path, version: &Version) -> Result<(), String> {
    if !app.is_dir() {
        return Err("the download doesn't contain md-wingman.app".into());
    }
    run(
        "codesign",
        &[
            os("--verify"),
            os("--strict"),
            os("--deep"),
            app.as_os_str(),
        ],
    )
    .map_err(|e| format!("the update's signature doesn't verify: {e}"))?;
    let plist = app.join("Contents/Info.plist");
    let field = |key: &str| -> Result<String, String> {
        let out = run(
            "plutil",
            &[
                os("-extract"),
                os(key),
                os("raw"),
                os("-o"),
                os("-"),
                plist.as_os_str(),
            ],
        )?;
        Ok(out.trim().to_string())
    };
    let id = field("CFBundleIdentifier")?;
    if id != BUNDLE_ID {
        return Err(format!("the update is a different app ({id})"));
    }
    let found = field("CFBundleShortVersionString")?;
    if found != version.to_string() {
        return Err(format!(
            "the update says it's version {found}, not {version}"
        ));
    }
    // Once releases are signed by a Developer ID team, only that team's
    // builds may replace them. Ad-hoc builds have no team to compare.
    let Some(ours) = std::env::current_exe()
        .ok()
        .and_then(|e| e.canonicalize().ok())
        .and_then(|e| bundle_of(&e))
        .and_then(|b| signing_team(&b))
    else {
        return Ok(());
    };
    match signing_team(app) {
        Some(theirs) if theirs == ours => Ok(()),
        theirs => Err(format!(
            "the update is signed by {}, not by this app's team {ours}",
            theirs.as_deref().unwrap_or("no team")
        )),
    }
}

fn signing_team(app: &Path) -> Option<String> {
    let out = std::process::Command::new("codesign")
        .args([os("-dv"), app.as_os_str()])
        .output()
        .ok()?;
    // codesign prints the details on stderr.
    team_identifier(&String::from_utf8_lossy(&out.stderr))
}

fn os(s: &str) -> &OsStr {
    OsStr::new(s)
}

fn run(program: &str, args: &[&OsStr]) -> Result<String, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            html_url: format!("https://github.com/deanberris/md-wingman/releases/tag/{tag}"),
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: name.to_string(),
                    browser_download_url: format!("https://example.invalid/{name}"),
                })
                .collect(),
        }
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[::core::prelude::v1::test]
    fn picks_the_newest_newer_release() {
        let zip = asset_name(&v("0.3.0"));
        let releases = [
            release("v0.2.0", true, &[]),
            release("v0.3.0", true, &[&zip, "SHA256SUMS.txt"]),
            release("v0.1.0", false, &[]),
            release("nightly", true, &[]),
        ];
        let found = newest(&releases, &v("0.1.0"), true).unwrap();
        assert_eq!(found.version, v("0.3.0"));
        assert!(found.page.ends_with("/v0.3.0"));
        let download = found.download.unwrap();
        assert_eq!(download.name, zip);
        assert!(download.sums_url.ends_with("/SHA256SUMS.txt"));
    }

    #[::core::prelude::v1::test]
    fn nothing_newer_means_no_update() {
        let releases = [release("v0.1.0", false, &[]), release("v0.0.9", false, &[])];
        assert_eq!(newest(&releases, &v("0.1.0"), true), None);
    }

    #[::core::prelude::v1::test]
    fn skips_drafts_and_optionally_prereleases() {
        let mut draft = release("v0.9.0", false, &[]);
        draft.draft = true;
        let releases = [
            draft,
            release("v0.3.0", true, &[]),
            release("v0.2.0", false, &[]),
        ];
        assert_eq!(
            newest(&releases, &v("0.1.0"), true).unwrap().version,
            v("0.3.0")
        );
        assert_eq!(
            newest(&releases, &v("0.1.0"), false).unwrap().version,
            v("0.2.0")
        );
    }

    #[::core::prelude::v1::test]
    fn semver_prereleases_sort_before_their_release() {
        let releases = [
            release("v0.2.0-beta.2", true, &[]),
            release("v0.2.0", true, &[]),
        ];
        assert_eq!(
            newest(&releases, &v("0.2.0-beta.1"), true).unwrap().version,
            v("0.2.0")
        );
        assert_eq!(newest(&releases, &v("0.2.0"), true), None);
    }

    #[::core::prelude::v1::test]
    fn a_release_without_the_zip_or_checksums_has_no_download() {
        let zip = asset_name(&v("0.2.0"));
        let releases = [release("v0.2.0", false, &[&zip])];
        let found = newest(&releases, &v("0.1.0"), true).unwrap();
        assert_eq!(found.download, None);
    }

    #[::core::prelude::v1::test]
    fn reads_checksums_from_shasum_output() {
        let hash = "ab".repeat(32);
        let sums = format!(
            "{hash}  md-wingman-0.2.0-macos-arm64.zip\n{}  md-wingman-0.2.0-macos-arm64.dmg\n",
            "cd".repeat(32)
        );
        assert_eq!(
            checksum_for(&sums, "md-wingman-0.2.0-macos-arm64.zip"),
            Some(hash.clone())
        );
        let binary = format!("{} *x.zip\n", hash.to_uppercase());
        assert_eq!(checksum_for(&binary, "x.zip"), Some(hash));
        assert_eq!(checksum_for(&sums, "other.zip"), None);
        assert_eq!(checksum_for("nonsense  x.zip\n", "x.zip"), None);
    }

    #[::core::prelude::v1::test]
    fn reads_the_team_from_codesign() {
        let adhoc = "Executable=/A.app/Contents/MacOS/a\nSignature=adhoc\nTeamIdentifier=not set\n";
        assert_eq!(team_identifier(adhoc), None);
        let signed =
            "Authority=Developer ID Application: X (ABCDE12345)\nTeamIdentifier=ABCDE12345\n";
        assert_eq!(team_identifier(signed).as_deref(), Some("ABCDE12345"));
    }

    #[::core::prelude::v1::test]
    fn finds_the_bundle_around_the_executable() {
        let exe = Path::new("/Applications/md-wingman.app/Contents/MacOS/md-wingman");
        assert_eq!(
            bundle_of(exe),
            Some(PathBuf::from("/Applications/md-wingman.app"))
        );
        assert_eq!(
            bundle_of(Path::new("/repo/target/release/md-wingman")),
            None
        );
    }

    #[::core::prelude::v1::test]
    fn knows_when_it_cant_replace_itself() {
        assert_eq!(
            stuck_reason(Path::new("/Applications/md-wingman.app")),
            None
        );
        assert_eq!(
            stuck_reason(Path::new("/Users/a/Applications/md-wingman.app")),
            None
        );
        assert!(stuck_reason(Path::new("/Volumes/md-wingman/md-wingman.app")).is_some());
        assert!(
            stuck_reason(Path::new(
                "/private/var/folders/x/AppTranslocation/1234/d/md-wingman.app"
            ))
            .is_some()
        );
        // An app kept on another disk is fine.
        assert_eq!(
            stuck_reason(Path::new("/Volumes/Data/Applications/md-wingman.app")),
            None
        );
    }
}
