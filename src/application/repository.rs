use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;

use crate::domain::change::ChangedFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub modified: SystemTime,
    pub len: u64,
}

pub trait Repository: Send + Sync {
    fn root(&self) -> &Path;
    fn branch_name(&self) -> Result<String>;
    fn changed_files(&self) -> Result<Vec<ChangedFile>>;
    fn source_files(&self, extension: &str) -> Result<Vec<PathBuf>>;
    fn head_content(&self, path: &Path) -> Result<Option<Vec<u8>>>;
    fn worktree_content(&self, path: &Path) -> Result<Option<Vec<u8>>>;
    fn worktree_stamp(&self, path: &Path) -> Option<FileStamp>;
    fn stage(&self, paths: &[&Path]) -> Result<()>;
    fn unstage(&self, paths: &[&Path]) -> Result<()>;
    fn stage_all(&self) -> Result<()>;
    fn unstage_all(&self) -> Result<()>;
    fn discard(&self, file: &ChangedFile) -> Result<()>;
    fn commit(&self, message: &str) -> Result<String>;
}
