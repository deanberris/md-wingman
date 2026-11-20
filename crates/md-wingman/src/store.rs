//! Fact-check results saved per document, so they come back when the file
//! is reopened. They live in the app's data directory (on macOS,
//! `~/Library/Application Support/md-wingman/fact-checks/`), keyed by the
//! document's path, so documents and their repositories stay clean.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::suggestions::Claim;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedClaim {
    pub claim: Claim,
    /// Where the quoted text was last seen, to re-attach it to the nearest
    /// match.
    pub anchor: usize,
    /// When it was checked, in seconds since the Unix epoch.
    pub checked_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedFile {
    /// The document these belong to, for anyone reading the file.
    document: PathBuf,
    claims: Vec<SavedClaim>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn store_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("WINGMAN_DATA_DIR") {
        return Some(PathBuf::from(dir).join("fact-checks"));
    }
    Some(dirs::data_dir()?.join("md-wingman").join("fact-checks"))
}

/// A stable file name for a document: FNV-1a of its canonical path.
fn file_for(document: &Path) -> Option<PathBuf> {
    let canonical = document
        .canonicalize()
        .unwrap_or_else(|_| document.to_path_buf());
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in canonical.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Some(store_dir()?.join(format!("{hash:016x}.json")))
}

pub fn load(document: &Path) -> Vec<SavedClaim> {
    let Some(path) = file_for(document) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<SavedFile>(&text).ok())
        .map(|f| f.claims)
        .unwrap_or_default()
}

/// Saves the claims for `document`, or removes the file when there are none.
pub fn save(document: &Path, claims: Vec<SavedClaim>) -> std::io::Result<()> {
    let Some(path) = file_for(document) else {
        return Ok(());
    };
    if claims.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = SavedFile {
        document: document.to_path_buf(),
        claims,
    };
    let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggestions::{Source, Verdict};

    #[test]
    fn saves_loads_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("WINGMAN_DATA_DIR", dir.path());
        let doc = dir.path().join("post.md");
        std::fs::write(&doc, "The tower opened in 1889.").unwrap();
        assert!(load(&doc).is_empty());

        let saved = SavedClaim {
            claim: Claim {
                quote: "opened in 1889".into(),
                verdict: Verdict::Supported,
                explanation: "Yes.".into(),
                sources: vec![Source {
                    title: "t".into(),
                    url: "https://x".into(),
                }],
            },
            anchor: 10,
            checked_at: 1,
        };
        save(&doc, vec![saved.clone()]).unwrap();
        assert_eq!(load(&doc), vec![saved]);
        save(&doc, Vec::new()).unwrap();
        assert!(load(&doc).is_empty());
        std::env::remove_var("WINGMAN_DATA_DIR");
    }
}
