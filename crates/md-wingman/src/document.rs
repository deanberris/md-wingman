//! The open document: where it lives on disk and whether it has unsaved edits.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Document {
    path: Option<PathBuf>,
    /// The text as last loaded or saved, to tell whether there are edits.
    saved_text: String,
}

impl Document {
    pub fn untitled() -> Self {
        Self::default()
    }

    /// Reads a document from disk. Returns it with its text.
    pub fn open(path: &Path) -> io::Result<(Self, String)> {
        let text = fs::read_to_string(path)?;
        let doc = Self {
            path: Some(path.to_owned()),
            saved_text: text.clone(),
        };
        Ok((doc, text))
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The name to show in the title bar.
    pub fn display_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
    }

    pub fn is_dirty(&self, current_text: &str) -> bool {
        self.saved_text != current_text
    }

    /// Writes `text` to `path` (or the document's own path) and remembers it.
    /// Writes to a temporary file first, so a failed save can't truncate the
    /// original.
    pub fn save(&mut self, text: &str, path: Option<&Path>) -> io::Result<()> {
        let path = match path.or(self.path.as_deref()) {
            Some(p) => p.to_owned(),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "document has no path yet",
                ));
            }
        };
        let tmp = path.with_extension(format!(
            "{}.md-wingman-tmp",
            path.extension()
                .map(|e| e.to_string_lossy())
                .unwrap_or_default()
        ));
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &path)?;
        self.path = Some(path);
        self.saved_text = text.to_owned();
        Ok(())
    }
}

/// Counts words the way a writer would: runs of non-whitespace.
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untitled_has_no_path_and_is_clean_when_empty() {
        let doc = Document::untitled();
        assert_eq!(doc.display_name(), "Untitled");
        assert!(!doc.is_dirty(""));
        assert!(doc.is_dirty("x"));
    }

    #[test]
    fn open_edit_save_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "# Hi\n").unwrap();

        let (mut doc, text) = Document::open(&path).unwrap();
        assert_eq!(text, "# Hi\n");
        assert_eq!(doc.display_name(), "note.md");
        assert!(!doc.is_dirty(&text));

        let edited = "# Hi\n\nMore.\n";
        assert!(doc.is_dirty(edited));
        doc.save(edited, None).unwrap();
        assert!(!doc.is_dirty(edited));
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        // No temp file left behind.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn save_untitled_needs_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = Document::untitled();
        assert!(doc.save("x", None).is_err());
        let path = dir.path().join("new.md");
        doc.save("x", Some(&path)).unwrap();
        assert_eq!(doc.path(), Some(path.as_path()));
    }

    #[test]
    fn counts_words() {
        assert_eq!(word_count("  The cat\nsat.  "), 3);
        assert_eq!(word_count(""), 0);
    }
}
