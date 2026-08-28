use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;

use crate::domain::change::ChangedFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub modified: SystemTime,
    pub len: u64,
}

/// A `base..head` comparison requested on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeSpec {
    pub base: String,
    pub head: String,
}

impl RangeSpec {
    /// Parses `base..head`, `base...head` or `base..`. Both forms are treated as
    /// merge-base semantics: reviewing your own work never wants the commits the
    /// base picked up after you branched off it.
    pub fn parse(text: &str) -> Option<Self> {
        let (base, head) = match text.split_once("...") {
            Some(split) => split,
            None => text.split_once("..")?,
        };
        // Only emptiness is checked here: dots are legal in refs (`v1.2.3`), and
        // git is the authority on whether the rest actually resolves.
        if base.is_empty() {
            return None;
        }
        let head = if head.is_empty() { DEFAULT_HEAD } else { head };
        Some(Self {
            base: base.to_string(),
            head: head.to_string(),
        })
    }

    pub fn label(&self) -> String {
        format!("{}…{}", self.base, self.head)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoMode {
    /// HEAD against the working tree, with staging and committing.
    Worktree,
    /// A committed range, reviewed read-only.
    Range(RangeSpec),
}

impl RepoMode {
    pub fn range(&self) -> Option<&RangeSpec> {
        match self {
            RepoMode::Worktree => None,
            RepoMode::Range(spec) => Some(spec),
        }
    }

    pub fn is_range(&self) -> bool {
        self.range().is_some()
    }
}

pub trait Repository: Send + Sync {
    fn root(&self) -> &Path;
    fn mode(&self) -> &RepoMode;
    fn branch_name(&self) -> Result<String>;
    /// Resolved commit of the reviewed head; `None` in worktree mode. The app
    /// watches this to know when a range's content has actually moved.
    fn review_head_id(&self) -> Option<String>;
    fn changed_files(&self) -> Result<Vec<ChangedFile>>;
    fn source_files(&self, extension: &str) -> Result<Vec<PathBuf>>;
    /// Content of the left-hand side: HEAD, or the merge-base in range mode.
    fn head_content(&self, path: &Path) -> Result<Option<Vec<u8>>>;
    /// Content of the right-hand side: the working tree, or the head ref in range mode.
    fn new_content(&self, path: &Path) -> Result<Option<Vec<u8>>>;
    /// `None` in range mode — a committed blob never goes stale under us.
    fn worktree_stamp(&self, path: &Path) -> Option<FileStamp>;
    fn stage(&self, paths: &[&Path]) -> Result<()>;
    fn unstage(&self, paths: &[&Path]) -> Result<()>;
    fn stage_all(&self) -> Result<()>;
    fn unstage_all(&self) -> Result<()>;
    fn discard(&self, file: &ChangedFile) -> Result<()>;
    fn commit(&self, message: &str) -> Result<String>;
}

const DEFAULT_HEAD: &str = "HEAD";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_and_three_dot_ranges_identically() {
        let two = RangeSpec::parse("main..feature").unwrap();
        let three = RangeSpec::parse("main...feature").unwrap();
        assert_eq!(two, three);
        assert_eq!(two.base, "main");
        assert_eq!(two.head, "feature");
    }

    #[test]
    fn defaults_an_empty_head_to_head() {
        let spec = RangeSpec::parse("main..").unwrap();
        assert_eq!(spec.head, "HEAD");
    }

    #[test]
    fn accepts_dotted_refs_like_version_tags() {
        let spec = RangeSpec::parse("v0.2.1..HEAD").unwrap();
        assert_eq!(spec.base, "v0.2.1");
        assert_eq!(spec.head, "HEAD");
        let spec = RangeSpec::parse("v1.0...v2.0").unwrap();
        assert_eq!(spec.base, "v1.0");
        assert_eq!(spec.head, "v2.0");
        let spec = RangeSpec::parse("v0.2.1..").unwrap();
        assert_eq!(spec.base, "v0.2.1");
        assert_eq!(spec.head, "HEAD");
    }

    #[test]
    fn rejects_non_ranges() {
        assert!(RangeSpec::parse("main").is_none());
        assert!(RangeSpec::parse("..main").is_none());
    }
}
