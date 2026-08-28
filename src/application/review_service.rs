use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Result, bail};

use super::repository::{FileStamp, RepoMode, Repository};
use crate::domain::change::{ChangedFile, StageState};
use crate::domain::diff::{SideBySideDiff, side_by_side};
use crate::domain::importance::{FileGroup, ImportancePolicy};
use crate::domain::python::parse_python;
use crate::domain::symbols::{DefinitionQuery, Symbol, SymbolIndex};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoSnapshot {
    pub branch: String,
    pub groups: Vec<FileGroup>,
    /// Commit the range currently resolves to; `None` in worktree mode.
    pub head_id: Option<String>,
}

impl RepoSnapshot {
    pub fn files(&self) -> impl Iterator<Item = &ChangedFile> {
        self.groups.iter().flat_map(|group| group.files.iter())
    }

    pub fn total(&self) -> usize {
        self.files().count()
    }

    pub fn staged_count(&self) -> usize {
        self.files().filter(|file| file.stage != StageState::Unstaged).count()
    }

    pub fn find(&self, path: &Path) -> Option<&ChangedFile> {
        self.files().find(|file| file.path == path)
    }

    pub fn find_mut(&mut self, path: &Path) -> Option<&mut ChangedFile> {
        self.groups
            .iter_mut()
            .flat_map(|group| group.files.iter_mut())
            .find(|file| file.path == path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffContent {
    Text(SideBySideDiff),
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub file: ChangedFile,
    pub old_text: String,
    pub new_text: String,
    pub content: DiffContent,
    pub stamp: Option<FileStamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeekSource {
    pub path: PathBuf,
    pub text: String,
    pub line: usize,
}

pub struct ReviewService {
    repo: Box<dyn Repository>,
    policy: ImportancePolicy,
    symbols: RwLock<SymbolIndex>,
    /// Range mode has no index to stage into, so "reviewed" lives here instead.
    /// Keyed by the blob that was reviewed: when you fix something and commit,
    /// only the files that actually changed come back unreviewed.
    reviewed: RwLock<HashMap<PathBuf, String>>,
}

impl ReviewService {
    pub fn new(repo: Box<dyn Repository>, policy: ImportancePolicy) -> Self {
        Self {
            repo,
            policy,
            symbols: RwLock::new(SymbolIndex::default()),
            reviewed: RwLock::new(HashMap::new()),
        }
    }

    pub fn mode(&self) -> &RepoMode {
        self.repo.mode()
    }

    pub fn is_range(&self) -> bool {
        self.repo.mode().is_range()
    }

    fn reviewed_read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<PathBuf, String>> {
        self.reviewed.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn reviewed_write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<PathBuf, String>> {
        self.reviewed.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn apply_review_marks(&self, files: &mut [ChangedFile]) {
        let marks = self.reviewed_read();
        for file in files {
            let reviewed = marks.get(&file.path).is_some_and(|marked| marked == &review_key(file));
            file.stage = if reviewed { StageState::Staged } else { StageState::Unstaged };
        }
    }

    /// Builds the whole index from scratch. In range mode every read is a
    /// `git show` subprocess, so this is meaningfully slower than the worktree's
    /// `fs::read` on Python-heavy repos — `git cat-file --batch` would fix that.
    /// It runs on a background thread behind the "indexing symbols…" indicator,
    /// so it degrades startup latency for go-to-definition rather than blocking.
    pub fn rebuild_symbol_index(&self) -> Result<usize> {
        let files = self.repo.source_files(PYTHON_EXTENSION)?;
        let mut index = SymbolIndex::default();
        for path in files {
            if let Ok(Some(bytes)) = self.repo.new_content(&path) {
                let text = String::from_utf8_lossy(&bytes);
                index.insert(path.clone(), parse_python(&path, &text));
            }
        }
        let count = index.file_count();
        *self.symbols_write() = index;
        Ok(count)
    }

    pub fn refresh_symbols<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        // Range mode reads content with `git show`, one subprocess per file, and
        // this runs on the UI thread for every changed file on every snapshot.
        // Committed content cannot change unless the head moves, and the app
        // rebuilds the whole index in the background when it does.
        if self.is_range() {
            return;
        }
        let mut index = self.symbols_write();
        for path in paths {
            if path.extension().and_then(|ext| ext.to_str()) != Some(PYTHON_EXTENSION) {
                continue;
            }
            match self.repo.new_content(path) {
                Ok(Some(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    index.insert(path.to_path_buf(), parse_python(path, &text));
                }
                _ => index.remove(path),
            }
        }
    }

    pub fn definitions(&self, file: &Path, word: &str, attribute_access: bool) -> Vec<Symbol> {
        self.symbols_read().resolve(DefinitionQuery {
            file,
            word,
            attribute_access,
        })
    }

    pub fn peek(&self, symbol: &Symbol) -> Result<PeekSource> {
        let Some(bytes) = self.repo.new_content(&symbol.path)? else {
            bail!("{} no longer exists", symbol.path.display());
        };
        Ok(PeekSource {
            path: symbol.path.clone(),
            text: to_display_text(Some(bytes)),
            line: symbol.line,
        })
    }

    fn symbols_read(&self) -> std::sync::RwLockReadGuard<'_, SymbolIndex> {
        self.symbols.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn symbols_write(&self) -> std::sync::RwLockWriteGuard<'_, SymbolIndex> {
        self.symbols.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn root(&self) -> &Path {
        self.repo.root()
    }

    pub fn repo_name(&self) -> String {
        self.repo
            .root()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.repo.root().display().to_string())
    }

    pub fn snapshot(&self) -> Result<RepoSnapshot> {
        let mut files = self.repo.changed_files()?;
        if self.is_range() {
            self.apply_review_marks(&mut files);
        }
        let branch = self.repo.branch_name().unwrap_or_else(|_| UNKNOWN_BRANCH.to_string());
        Ok(RepoSnapshot {
            branch,
            groups: self.policy.group(files),
            head_id: self.repo.review_head_id(),
        })
    }

    pub fn load_diff(&self, file: &ChangedFile) -> Result<FileDiff> {
        let old = if file.exists_in_head() {
            self.repo.head_content(file.head_path())?
        } else {
            None
        };
        let new = if file.exists_in_worktree() {
            self.repo.new_content(&file.path)?
        } else {
            None
        };
        let stamp = self.repo.worktree_stamp(&file.path);
        if is_binary(&old) || is_binary(&new) {
            return Ok(FileDiff {
                file: file.clone(),
                old_text: String::new(),
                new_text: String::new(),
                content: DiffContent::Binary,
                stamp,
            });
        }
        let old_text = to_display_text(old);
        let new_text = to_display_text(new);
        let diff = side_by_side(&old_text, &new_text);
        Ok(FileDiff {
            file: file.clone(),
            old_text,
            new_text,
            content: DiffContent::Text(diff),
            stamp,
        })
    }

    pub fn is_stale(&self, path: &Path, stamp: Option<FileStamp>) -> bool {
        self.repo.worktree_stamp(path) != stamp
    }

    pub fn toggle_stage(&self, file: &ChangedFile) -> Result<StageState> {
        if self.is_range() {
            let mut marks = self.reviewed_write();
            if file.is_fully_staged() {
                marks.remove(&file.path);
                return Ok(StageState::Unstaged);
            }
            marks.insert(file.path.clone(), review_key(file));
            return Ok(StageState::Staged);
        }
        if file.is_fully_staged() {
            self.repo.unstage(&file.all_paths())?;
            Ok(StageState::Unstaged)
        } else {
            self.repo.stage(&file.all_paths())?;
            Ok(StageState::Staged)
        }
    }

    pub fn stage_all(&self) -> Result<()> {
        if self.is_range() {
            let files = self.repo.changed_files()?;
            let mut marks = self.reviewed_write();
            for file in files {
                marks.insert(file.path.clone(), review_key(&file));
            }
            return Ok(());
        }
        self.repo.stage_all()
    }

    pub fn unstage_all(&self) -> Result<()> {
        if self.is_range() {
            self.reviewed_write().clear();
            return Ok(());
        }
        self.repo.unstage_all()
    }

    pub fn discard(&self, file: &ChangedFile) -> Result<()> {
        self.repo.discard(file)
    }

    pub fn commit(&self, message: &str) -> Result<String> {
        self.repo.commit(message.trim())
    }
}

/// What a review mark is keyed on. Range mode always supplies a content id; the
/// sentinel only covers the unreachable case so marking and matching can never
/// disagree — when they do, marks silently fail to stick.
fn review_key(file: &ChangedFile) -> String {
    file.content_id.clone().unwrap_or_else(|| UNKNOWN_CONTENT.to_string())
}

fn is_binary(bytes: &Option<Vec<u8>>) -> bool {
    bytes
        .as_ref()
        .is_some_and(|content| content.iter().take(BINARY_PROBE_BYTES).any(|&byte| byte == 0))
}

fn to_display_text(bytes: Option<Vec<u8>>) -> String {
    bytes
        .map(|content| String::from_utf8_lossy(&content).replace('\t', TAB_REPLACEMENT))
        .unwrap_or_default()
}

const BINARY_PROBE_BYTES: usize = 8000;
const PYTHON_EXTENSION: &str = "py";
const TAB_REPLACEMENT: &str = "    ";
const UNKNOWN_BRANCH: &str = "?";
const UNKNOWN_CONTENT: &str = "unknown";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::repository::RangeSpec;
    use crate::domain::change::{ChangeKind, LineStats};
    use std::sync::{Arc, Mutex};

    type SharedFiles = Arc<Mutex<Vec<ChangedFile>>>;

    struct FakeRepo {
        mode: RepoMode,
        files: SharedFiles,
    }

    impl FakeRepo {
        fn range(files: Vec<ChangedFile>) -> (Self, SharedFiles) {
            let shared: SharedFiles = Arc::new(Mutex::new(files));
            let repo = Self {
                mode: RepoMode::Range(RangeSpec { base: "main".into(), head: "feature".into() }),
                files: Arc::clone(&shared),
            };
            (repo, shared)
        }
    }

    impl Repository for FakeRepo {
        fn root(&self) -> &Path {
            Path::new("/fake")
        }
        fn mode(&self) -> &RepoMode {
            &self.mode
        }
        fn branch_name(&self) -> Result<String> {
            Ok("main…feature".to_string())
        }
        fn review_head_id(&self) -> Option<String> {
            Some("fake-head".to_string())
        }
        fn changed_files(&self) -> Result<Vec<ChangedFile>> {
            Ok(self.files.lock().unwrap().clone())
        }
        fn source_files(&self, _extension: &str) -> Result<Vec<PathBuf>> {
            Ok(Vec::new())
        }
        fn head_content(&self, _path: &Path) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }
        fn new_content(&self, _path: &Path) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }
        fn worktree_stamp(&self, _path: &Path) -> Option<FileStamp> {
            None
        }
        fn stage(&self, _paths: &[&Path]) -> Result<()> {
            bail!("read-only")
        }
        fn unstage(&self, _paths: &[&Path]) -> Result<()> {
            bail!("read-only")
        }
        fn stage_all(&self) -> Result<()> {
            bail!("read-only")
        }
        fn unstage_all(&self) -> Result<()> {
            bail!("read-only")
        }
        fn discard(&self, _file: &ChangedFile) -> Result<()> {
            bail!("read-only")
        }
        fn commit(&self, _message: &str) -> Result<String> {
            bail!("read-only")
        }
    }

    fn file(path: &str, content_id: Option<&str>, kind: ChangeKind) -> ChangedFile {
        ChangedFile {
            path: PathBuf::from(path),
            kind,
            stage: StageState::Unstaged,
            stats: LineStats::default(),
            content_id: content_id.map(str::to_string),
        }
    }

    fn service(repo: FakeRepo) -> ReviewService {
        ReviewService::new(Box::new(repo), ImportancePolicy::new(Vec::new(), Vec::new()))
    }

    fn stage_of(service: &ReviewService, path: &str) -> StageState {
        service.snapshot().unwrap().find(Path::new(path)).unwrap().stage
    }

    #[test]
    fn marking_a_file_read_survives_the_next_snapshot() {
        let modified = file("a.py", Some("base-1:blob-1"), ChangeKind::Modified);
        let (repo, _files) = FakeRepo::range(vec![modified.clone()]);
        let service = service(repo);
        assert_eq!(stage_of(&service, "a.py"), StageState::Unstaged);

        service.toggle_stage(&modified).unwrap();
        assert_eq!(stage_of(&service, "a.py"), StageState::Staged);
    }

    #[test]
    fn marking_a_deleted_file_read_survives_the_next_snapshot() {
        // Deleted files have no blob; marking and matching must agree on the
        // sentinel or the mark silently reverts on the next poll.
        let deleted = file("gone.py", None, ChangeKind::Deleted);
        let (repo, _files) = FakeRepo::range(vec![deleted.clone()]);
        let service = service(repo);

        service.toggle_stage(&deleted).unwrap();
        assert_eq!(stage_of(&service, "gone.py"), StageState::Staged);
    }

    #[test]
    fn a_moved_merge_base_drops_the_mark_even_when_the_head_blob_is_untouched() {
        // origin/main advances past a commit touching a.py: the head blob is
        // unchanged but the diff on screen is different, so the mark must drop.
        let file_a = file("a.py", Some("base-1:blob-1"), ChangeKind::Modified);
        let (repo, files) = FakeRepo::range(vec![file_a.clone()]);
        let service = service(repo);
        service.toggle_stage(&file_a).unwrap();
        assert_eq!(stage_of(&service, "a.py"), StageState::Staged);

        files.lock().unwrap()[0].content_id = Some("base-1b:blob-1".to_string());
        assert_eq!(stage_of(&service, "a.py"), StageState::Unstaged);
    }

    #[test]
    fn a_changed_blob_drops_the_mark_but_leaves_others_alone() {
        let changing = file("a.py", Some("base-1:blob-1"), ChangeKind::Modified);
        let steady = file("b.py", Some("base-2:blob-2"), ChangeKind::Modified);
        let (repo, files) = FakeRepo::range(vec![changing.clone(), steady.clone()]);
        let service = service(repo);
        service.toggle_stage(&changing).unwrap();
        service.toggle_stage(&steady).unwrap();
        assert_eq!(stage_of(&service, "a.py"), StageState::Staged);
        assert_eq!(stage_of(&service, "b.py"), StageState::Staged);

        // Simulate a fixup commit that touches only a.py.
        files.lock().unwrap()[0].content_id = Some("base-1:blob-1b".to_string());
        assert_eq!(stage_of(&service, "a.py"), StageState::Unstaged, "changed file comes back unread");
        assert_eq!(stage_of(&service, "b.py"), StageState::Staged, "untouched file stays read");
    }

    #[test]
    fn mark_all_and_clear_cover_blobless_files() {
        let (repo, _files) = FakeRepo::range(vec![
            file("a.py", Some("base-1:blob-1"), ChangeKind::Modified),
            file("gone.py", None, ChangeKind::Deleted),
        ]);
        let service = service(repo);
        service.stage_all().unwrap();
        assert_eq!(service.snapshot().unwrap().staged_count(), 2, "mark-all must cover deleted files too");

        service.unstage_all().unwrap();
        assert_eq!(service.snapshot().unwrap().staged_count(), 0);
    }
}
