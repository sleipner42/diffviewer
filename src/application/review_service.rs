use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Result, bail};

use super::repository::{FileStamp, Repository};
use crate::domain::change::{ChangedFile, StageState};
use crate::domain::diff::{SideBySideDiff, side_by_side};
use crate::domain::importance::{FileGroup, ImportancePolicy};
use crate::domain::python::parse_python;
use crate::domain::symbols::{DefinitionQuery, Symbol, SymbolIndex};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoSnapshot {
    pub branch: String,
    pub groups: Vec<FileGroup>,
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
}

impl ReviewService {
    pub fn new(repo: Box<dyn Repository>, policy: ImportancePolicy) -> Self {
        Self {
            repo,
            policy,
            symbols: RwLock::new(SymbolIndex::default()),
        }
    }

    pub fn rebuild_symbol_index(&self) -> Result<usize> {
        let files = self.repo.source_files(PYTHON_EXTENSION)?;
        let mut index = SymbolIndex::default();
        for path in files {
            if let Ok(Some(bytes)) = self.repo.worktree_content(&path) {
                let text = String::from_utf8_lossy(&bytes);
                index.insert(path.clone(), parse_python(&path, &text));
            }
        }
        let count = index.file_count();
        *self.symbols_write() = index;
        Ok(count)
    }

    pub fn refresh_symbols<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        let mut index = self.symbols_write();
        for path in paths {
            if path.extension().and_then(|ext| ext.to_str()) != Some(PYTHON_EXTENSION) {
                continue;
            }
            match self.repo.worktree_content(path) {
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
        let Some(bytes) = self.repo.worktree_content(&symbol.path)? else {
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
        let files = self.repo.changed_files()?;
        let branch = self.repo.branch_name().unwrap_or_else(|_| UNKNOWN_BRANCH.to_string());
        Ok(RepoSnapshot {
            branch,
            groups: self.policy.group(files),
        })
    }

    pub fn load_diff(&self, file: &ChangedFile) -> Result<FileDiff> {
        let old = if file.exists_in_head() {
            self.repo.head_content(file.head_path())?
        } else {
            None
        };
        let new = if file.exists_in_worktree() {
            self.repo.worktree_content(&file.path)?
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
        if file.is_fully_staged() {
            self.repo.unstage(&file.all_paths())?;
            Ok(StageState::Unstaged)
        } else {
            self.repo.stage(&file.all_paths())?;
            Ok(StageState::Staged)
        }
    }

    pub fn stage_all(&self) -> Result<()> {
        self.repo.stage_all()
    }

    pub fn unstage_all(&self) -> Result<()> {
        self.repo.unstage_all()
    }

    pub fn discard(&self, file: &ChangedFile) -> Result<()> {
        self.repo.discard(file)
    }

    pub fn commit(&self, message: &str) -> Result<String> {
        self.repo.commit(message.trim())
    }
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
