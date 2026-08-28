use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: PathBuf },
    Untracked,
}

impl ChangeKind {
    pub fn label(&self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Renamed { .. } => "renamed",
            ChangeKind::Untracked => "new",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageState {
    Unstaged,
    Staged,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LineStats {
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub kind: ChangeKind,
    pub stage: StageState,
    pub stats: LineStats,
    /// Identity of *both* sides of this file's diff in range mode (`src:dst`
    /// blobs); `None` in worktree mode. Review marks are keyed by it, so a mark
    /// drops when either side really changes — including when the merge base
    /// moves under a file whose head blob is untouched.
    pub content_id: Option<String>,
}

impl ChangedFile {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn directory(&self) -> String {
        self.path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn is_fully_staged(&self) -> bool {
        self.stage == StageState::Staged
    }

    pub fn exists_in_head(&self) -> bool {
        !matches!(self.kind, ChangeKind::Added | ChangeKind::Untracked)
    }

    pub fn exists_in_worktree(&self) -> bool {
        self.kind != ChangeKind::Deleted
    }

    pub fn head_path(&self) -> &Path {
        match &self.kind {
            ChangeKind::Renamed { from } => from,
            _ => &self.path,
        }
    }

    pub fn all_paths(&self) -> Vec<&Path> {
        match &self.kind {
            ChangeKind::Renamed { from } => vec![from.as_path(), self.path.as_path()],
            _ => vec![self.path.as_path()],
        }
    }

    pub fn same_content_identity(&self, other: &ChangedFile) -> bool {
        self.path == other.path && self.kind == other.kind && self.stats == other.stats && self.content_id == other.content_id
    }
}
