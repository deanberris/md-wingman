//! Crash reports: one small text file per crash, ready to attach to an issue.
//!
//! Logging a record under the [`TARGET`] target writes a report to
//! `crashes/` beside the log, with the app and system, what happened (for a
//! panic, its stack trace) and the few hundred log lines before it. The
//! panic hook does this for the app; the assistant client does it when the
//! service dies. Like the log, a report never holds secrets or writing.
//!
//! At the next launch, [`unseen`] lists the reports written since the app
//! last looked, plus any macOS crash reports for md-wingman (from crashes a
//! panic hook can't catch, such as a segfault), so the app can mention them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The log target that turns a record into a crash report.
pub const TARGET: &str = "crash";

/// Where to file a report.
pub const ISSUES_URL: &str = "https://github.com/deanberris/md-wingman/issues/new";

/// How many reports are kept; older ones are deleted.
const KEEP_REPORTS: usize = 10;

/// Remembers when the app last looked for new reports.
const SEEN_MARKER: &str = ".seen";

/// `crashes/` beside the log at `log_path`.
pub fn dir_for(log_path: &Path) -> PathBuf {
    log_path.parent().unwrap_or(Path::new(".")).join("crashes")
}

/// The crash report folder, once logging is set up.
pub fn dir() -> Option<PathBuf> {
    crate::path().map(dir_for)
}

/// Writes a crash report for `message` (already scrubbed) and returns its path.
pub fn write_report(message: &str) -> io::Result<PathBuf> {
    let dir = dir().ok_or_else(|| io::Error::other("logging isn't set up"))?;
    write_report_in(
        &dir,
        crate::app(),
        message,
        &crate::recent_lines(),
        SystemTime::now(),
    )
}

pub(crate) fn write_report_in(
    dir: &Path,
    app: &str,
    message: &str,
    recent: &[String],
    now: SystemTime,
) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = crate::timestamp(now);
    // Colons aren't welcome in file names on macOS.
    let mut path = dir.join(format!("crash-{}.txt", stamp.replace(':', "-")));
    let mut n = 1;
    while path.exists() {
        n += 1;
        path = dir.join(format!("crash-{}-{n}.txt", stamp.replace(':', "-")));
    }
    let app = if app.is_empty() { "md-wingman" } else { app };
    let report = format!(
        "md-wingman crash report\n\
         \n\
         When: {stamp}\n\
         App:  {app}\n\
         \n\
         To report it, attach this file to a new issue at {ISSUES_URL}\n\
         (Help ▸ Report a Problem… opens that page). It holds no keys or tokens, and none of your writing.\n\
         \n\
         What happened\n\
         -------------\n\
         {message}\n\
         \n\
         The last {} log lines before it\n\
         -------------------------------\n\
         {}\n",
        recent.len(),
        recent.join("\n")
    );
    fs::write(&path, report)?;
    prune(dir);
    Ok(path)
}

/// The reports in `dir`, oldest first.
pub fn reports_in(dir: &Path) -> Vec<PathBuf> {
    let mut reports: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    // The names start with the time, so they sort in order.
    reports.sort();
    reports
}

fn prune(dir: &Path) {
    let reports = reports_in(dir);
    let extra = reports.len().saturating_sub(KEEP_REPORTS);
    for old in &reports[..extra] {
        let _ = fs::remove_file(old);
    }
}

/// macOS's own crash reports, for crashes no panic hook sees.
fn system_report_dirs() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    dirs::home_dir()
        .map(|h| vec![h.join("Library/Logs/DiagnosticReports")])
        .unwrap_or_default()
}

/// Crash reports written since the app last asked, its own and macOS's,
/// oldest first. Asking marks them seen.
pub fn unseen() -> Vec<PathBuf> {
    match dir() {
        Some(dir) => unseen_in(&dir, &system_report_dirs()),
        None => Vec::new(),
    }
}

pub fn unseen_in(dir: &Path, system_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let marker = dir.join(SEEN_MARKER);
    let seen = fs::metadata(&marker).and_then(|m| m.modified()).ok();
    let newer = |p: &Path| {
        let modified = fs::metadata(p).and_then(|m| m.modified()).ok();
        match (seen, modified) {
            (Some(seen), Some(modified)) => modified > seen,
            // The first time, every report of ours counts.
            (None, _) => true,
            (Some(_), None) => false,
        }
    };
    let mut found: Vec<PathBuf> = reports_in(dir).into_iter().filter(|p| newer(p)).collect();
    // macOS keeps reports for every app: only ours, and only since we last
    // looked (the first time, there's no telling what's old).
    if seen.is_some() {
        for system in system_dirs {
            found.extend(
                fs::read_dir(system)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                            n.starts_with("md-wingman")
                                && (n.ends_with(".ips") || n.ends_with(".crash"))
                        })
                    })
                    .filter(|p| newer(p)),
            );
        }
    }
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(&marker, crate::timestamp(SystemTime::now()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn writes_reports_and_keeps_the_last_ten() {
        let dir = tempfile::tempdir().unwrap();
        let crashes = dir.path().join("crashes");
        let recent = vec!["line one".to_string(), "line two".to_string()];
        for n in 0..12u64 {
            let at = UNIX_EPOCH + Duration::from_secs(1_795_256_727 + n);
            let path = write_report_in(&crashes, "md-wingman 0.1.0", "boom", &recent, at).unwrap();
            assert!(path.to_str().unwrap().ends_with(".txt"));
            assert!(!path.file_name().unwrap().to_str().unwrap().contains(':'));
        }
        let reports = reports_in(&crashes);
        assert_eq!(reports.len(), KEEP_REPORTS);
        // The oldest two went.
        assert!(
            reports[0].to_str().unwrap().contains("10-25-29"),
            "{reports:?}"
        );
        let text = fs::read_to_string(&reports[0]).unwrap();
        assert!(text.contains("App:  md-wingman 0.1.0"), "{text}");
        assert!(text.contains("boom"), "{text}");
        assert!(text.contains("line one\nline two"), "{text}");
        assert!(text.contains(ISSUES_URL), "{text}");
    }

    #[test]
    fn two_crashes_in_one_millisecond_both_get_reports() {
        let dir = tempfile::tempdir().unwrap();
        let at = UNIX_EPOCH + Duration::from_secs(1_795_256_727);
        let a = write_report_in(dir.path(), "", "a", &[], at).unwrap();
        let b = write_report_in(dir.path(), "", "b", &[], at).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn finds_new_reports_including_macos_ones() {
        let dir = tempfile::tempdir().unwrap();
        let crashes = dir.path().join("crashes");
        let system = dir.path().join("DiagnosticReports");
        fs::create_dir_all(&system).unwrap();
        fs::write(system.join("md-wingman-old.ips"), "old").unwrap();
        // The first look marks everything seen, and ignores macOS's old ones.
        assert!(unseen_in(&crashes, std::slice::from_ref(&system)).is_empty());
        std::thread::sleep(Duration::from_millis(20));
        fs::write(system.join("md-wingman-2026-11-21.ips"), "segfault").unwrap();
        fs::write(system.join("OtherApp-2026-11-21.ips"), "not ours").unwrap();
        let ours = write_report_in(&crashes, "", "panic", &[], SystemTime::now()).unwrap();
        let found = unseen_in(&crashes, std::slice::from_ref(&system));
        assert_eq!(
            found,
            vec![ours, system.join("md-wingman-2026-11-21.ips")],
            "{found:?}"
        );
        assert!(unseen_in(&crashes, &[system]).is_empty());
    }
}
