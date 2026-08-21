use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::application::repository::{FileStamp, Repository};
use crate::domain::change::{ChangeKind, ChangedFile, LineStats, StageState};

pub struct GitCli {
    root: PathBuf,
}

impl GitCli {
    pub fn discover(start: &Path) -> Result<Self> {
        let output = Command::new(GIT)
            .arg("-C")
            .arg(start)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("failed to run git")?;
        if !output.status.success() {
            bail!("{} is not inside a git repository", start.display());
        }
        let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        Ok(Self { root })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(GIT);
        command.arg("-C").arg(&self.root);
        command
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut command = self.command();
        command.args(args);
        execute(command)
    }

    fn run_read_only(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut command = self.command();
        command.env("GIT_OPTIONAL_LOCKS", "0");
        command.args(args);
        execute(command)
    }

    fn run_with_paths(&self, args: &[&str], paths: &[&Path]) -> Result<()> {
        let mut command = self.command();
        command.args(args).arg("--").args(paths);
        execute(command).map(|_| ())
    }

    fn has_head(&self) -> bool {
        self.run_read_only(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok()
    }

    fn diff_base(&self) -> &'static str {
        if self.has_head() { "HEAD" } else { EMPTY_TREE }
    }

    fn line_stats(&self) -> Result<HashMap<PathBuf, LineStats>> {
        let base = self.diff_base();
        let output = self.run_read_only(&["diff", base, "--numstat", "-z", "-M"])?;
        Ok(parse_numstat(&output))
    }

    fn untracked_stats(&self, path: &Path) -> LineStats {
        let Ok(content) = std::fs::read(self.root.join(path)) else {
            return LineStats::default();
        };
        if content.iter().take(BINARY_PROBE_BYTES).any(|&byte| byte == 0) {
            return LineStats::default();
        }
        let newlines = content.iter().filter(|&&byte| byte == b'\n').count();
        let trailing = usize::from(!content.is_empty() && !content.ends_with(b"\n"));
        LineStats { added: newlines + trailing, removed: 0 }
    }
}

impl Repository for GitCli {
    fn root(&self) -> &Path {
        &self.root
    }

    fn branch_name(&self) -> Result<String> {
        if let Ok(output) = self.run_read_only(&["symbolic-ref", "--short", "-q", "HEAD"]) {
            return Ok(String::from_utf8_lossy(&output).trim().to_string());
        }
        let output = self.run_read_only(&["rev-parse", "--short", "HEAD"])?;
        Ok(format!("detached @ {}", String::from_utf8_lossy(&output).trim()))
    }

    fn changed_files(&self) -> Result<Vec<ChangedFile>> {
        let status = self.run_read_only(&["status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
        let stats = self.line_stats()?;
        let files = parse_porcelain_v2(&status)
            .into_iter()
            .map(|entry| {
                let stats = match entry.kind {
                    ChangeKind::Untracked => self.untracked_stats(&entry.path),
                    _ => stats.get(&entry.path).copied().unwrap_or_default(),
                };
                ChangedFile {
                    path: entry.path,
                    kind: entry.kind,
                    stage: entry.stage,
                    stats,
                }
            })
            .collect();
        Ok(files)
    }

    fn source_files(&self, extension: &str) -> Result<Vec<PathBuf>> {
        let pattern = format!("*.{extension}");
        let output = self.run_read_only(&[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            &pattern,
        ])?;
        let mut files: Vec<PathBuf> = output
            .split(|&byte| byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| PathBuf::from(String::from_utf8_lossy(entry).into_owned()))
            .collect();
        files.sort();
        files.dedup();
        Ok(files)
    }

    fn head_content(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        let spec = format!("{}:{}", self.diff_base(), path.to_string_lossy());
        match self.run_read_only(&["show", &spec]) {
            Ok(content) => Ok(Some(content)),
            Err(_) => Ok(None),
        }
    }

    fn worktree_content(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.root.join(path)) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    fn worktree_stamp(&self, path: &Path) -> Option<FileStamp> {
        let metadata = std::fs::metadata(self.root.join(path)).ok()?;
        Some(FileStamp {
            modified: metadata.modified().ok()?,
            len: metadata.len(),
        })
    }

    fn stage(&self, paths: &[&Path]) -> Result<()> {
        self.run_with_paths(&["add", "-A"], paths)
    }

    fn unstage(&self, paths: &[&Path]) -> Result<()> {
        if self.has_head() {
            self.run_with_paths(&["reset", "-q", "HEAD"], paths)
        } else {
            self.run_with_paths(&["rm", "-r", "-q", "--cached"], paths)
        }
    }

    fn stage_all(&self) -> Result<()> {
        self.run(&["add", "-A"]).map(|_| ())
    }

    fn unstage_all(&self) -> Result<()> {
        if self.has_head() {
            self.run(&["reset", "-q"]).map(|_| ())
        } else {
            self.run(&["rm", "-r", "-q", "--cached", "."]).map(|_| ())
        }
    }

    fn discard(&self, file: &ChangedFile) -> Result<()> {
        match file.kind {
            ChangeKind::Untracked => std::fs::remove_file(self.root.join(&file.path))
                .with_context(|| format!("deleting {}", file.path.display())),
            _ => {
                let base = self.diff_base();
                let source = format!("--source={base}");
                self.run_with_paths(&["restore", &source, "--staged", "--worktree"], &file.all_paths())
            }
        }
    }

    fn commit(&self, message: &str) -> Result<String> {
        self.run(&["commit", "-q", "-m", message])?;
        let output = self.run_read_only(&["log", "-1", "--format=%h %s"])?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
    }
}

struct StatusEntry {
    path: PathBuf,
    kind: ChangeKind,
    stage: StageState,
}

fn parse_porcelain_v2(bytes: &[u8]) -> Vec<StatusEntry> {
    let mut tokens = bytes
        .split(|&byte| byte == 0)
        .map(|token| String::from_utf8_lossy(token).into_owned());
    let mut entries = Vec::new();
    while let Some(entry) = tokens.next() {
        let Some((tag, rest)) = entry.split_once(' ') else {
            continue;
        };
        match tag {
            "1" => {
                let fields: Vec<&str> = rest.splitn(ORDINARY_FIELDS, ' ').collect();
                if let [xy, .., path] = fields.as_slice() {
                    entries.push(tracked_entry(xy, path, None));
                }
            }
            "2" => {
                let fields: Vec<&str> = rest.splitn(RENAME_FIELDS, ' ').collect();
                let from = tokens.next().map(PathBuf::from);
                if let [xy, .., path] = fields.as_slice() {
                    entries.push(tracked_entry(xy, path, from));
                }
            }
            "u" => {
                let fields: Vec<&str> = rest.splitn(UNMERGED_FIELDS, ' ').collect();
                if let [_, .., path] = fields.as_slice() {
                    entries.push(StatusEntry {
                        path: PathBuf::from(path),
                        kind: ChangeKind::Modified,
                        stage: StageState::Unstaged,
                    });
                }
            }
            "?" => entries.push(StatusEntry {
                path: PathBuf::from(rest),
                kind: ChangeKind::Untracked,
                stage: StageState::Unstaged,
            }),
            _ => {}
        }
    }
    entries
}

fn tracked_entry(xy: &str, path: &str, from: Option<PathBuf>) -> StatusEntry {
    let mut codes = xy.chars();
    let index = codes.next().unwrap_or('.');
    let worktree = codes.next().unwrap_or('.');
    let stage = match (index != '.', worktree != '.') {
        (true, true) => StageState::Partial,
        (true, false) => StageState::Staged,
        _ => StageState::Unstaged,
    };
    let kind = match from {
        Some(from) => ChangeKind::Renamed { from },
        None if worktree == 'D' || index == 'D' => ChangeKind::Deleted,
        None if index == 'A' => ChangeKind::Added,
        None => ChangeKind::Modified,
    };
    StatusEntry {
        path: PathBuf::from(path),
        kind,
        stage,
    }
}

fn parse_numstat(bytes: &[u8]) -> HashMap<PathBuf, LineStats> {
    let mut tokens = bytes
        .split(|&byte| byte == 0)
        .map(|token| String::from_utf8_lossy(token).into_owned());
    let mut stats = HashMap::new();
    while let Some(record) = tokens.next() {
        if record.is_empty() {
            continue;
        }
        let mut parts = record.splitn(3, '\t');
        let added = parse_count(parts.next());
        let removed = parse_count(parts.next());
        let inline_path = parts.next().unwrap_or("");
        let path = if inline_path.is_empty() {
            let _renamed_from = tokens.next();
            tokens.next().unwrap_or_default()
        } else {
            inline_path.to_string()
        };
        stats.insert(PathBuf::from(path), LineStats { added, removed });
    }
    stats
}

fn parse_count(value: Option<&str>) -> usize {
    value.and_then(|count| count.parse().ok()).unwrap_or(0)
}

fn execute(mut command: Command) -> Result<Vec<u8>> {
    let output = command.output().context("failed to run git")?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("{}", if stderr.is_empty() { "git command failed".to_string() } else { stderr })
    }
}

const GIT: &str = "git";
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const ORDINARY_FIELDS: usize = 8;
const RENAME_FIELDS: usize = 9;
const UNMERGED_FIELDS: usize = 10;
const BINARY_PROBE_BYTES: usize = 8000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ordinary_rename_and_untracked_entries() {
        let raw = b"1 .M N... 100644 100644 100644 abc def src/a.py\0\
2 R. N... 100644 100644 100644 abc def R100 src/new name.py\0src/old.py\0\
? notes.txt\0";
        let entries = parse_porcelain_v2(raw);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, PathBuf::from("src/a.py"));
        assert_eq!(entries[0].kind, ChangeKind::Modified);
        assert_eq!(entries[0].stage, StageState::Unstaged);
        assert_eq!(entries[1].path, PathBuf::from("src/new name.py"));
        assert_eq!(entries[1].kind, ChangeKind::Renamed { from: PathBuf::from("src/old.py") });
        assert_eq!(entries[1].stage, StageState::Staged);
        assert_eq!(entries[2].kind, ChangeKind::Untracked);
    }

    #[test]
    fn parses_numstat_with_renames_and_binaries() {
        let raw = b"3\t1\tsrc/a.py\0-\t-\timg.png\00\t0\t\0old.py\0new.py\0";
        let stats = parse_numstat(raw);
        assert_eq!(stats[&PathBuf::from("src/a.py")], LineStats { added: 3, removed: 1 });
        assert_eq!(stats[&PathBuf::from("img.png")], LineStats::default());
        assert!(stats.contains_key(&PathBuf::from("new.py")));
    }
}
