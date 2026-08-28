use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

use crate::application::repository::{FileStamp, GitRef, RangeSpec, RefKind, RepoMode, Repository};
use crate::domain::change::{ChangeKind, ChangedFile, LineStats, StageState};

pub struct GitCli {
    root: PathBuf,
    mode: RepoMode,
}

impl GitCli {
    pub fn discover(start: &Path) -> Result<Self> {
        Ok(Self {
            root: toplevel(start)?,
            mode: RepoMode::Worktree,
        })
    }

    /// Opens a committed range. Both endpoints are resolved once here so a typo
    /// fails at startup rather than showing an empty diff.
    pub fn discover_range(start: &Path, spec: RangeSpec) -> Result<Self> {
        let repo = Self {
            root: toplevel(start)?,
            mode: RepoMode::Worktree,
        };
        for rev in [&spec.base, &spec.head] {
            repo.resolve(rev).map_err(|_| anyhow!("{rev} is not a valid revision"))?;
        }
        repo.merge_base(&spec.base, &spec.head)
            .map_err(|_| anyhow!("{} and {} have no common ancestor", spec.base, spec.head))?;
        Ok(Self {
            root: repo.root,
            mode: RepoMode::Range(spec),
        })
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

    fn resolve(&self, rev: &str) -> Result<String> {
        let output = self.run_read_only(&["rev-parse", "--verify", "-q", &format!("{rev}^{{commit}}")])?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
    }

    fn merge_base(&self, base: &str, head: &str) -> Result<String> {
        let output = self.run_read_only(&["merge-base", base, head])?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
    }

    /// Left-hand side of the diff. In range mode this is the merge-base, which is
    /// what `base...head` means — it is re-resolved per call so the view tracks
    /// new commits on either endpoint.
    ///
    /// A merge-base failure must be an error, never a fallback: diffing the empty
    /// tree against the head still *succeeds*, so degrading here would silently
    /// redraw the review as "every file was added" when the base branch is
    /// deleted or pruned out from under an open tab.
    fn diff_base(&self) -> Result<String> {
        match self.mode.range() {
            Some(spec) => self
                .merge_base(&spec.base, &spec.head)
                .with_context(|| format!("cannot resolve the merge base of {} and {}", spec.base, spec.head)),
            None if self.has_head() => Ok("HEAD".to_string()),
            None => Ok(EMPTY_TREE.to_string()),
        }
    }

    /// Right-hand side. `None` means the working tree.
    fn diff_head(&self) -> Option<&str> {
        self.mode.range().map(|spec| spec.head.as_str())
    }

    fn line_stats(&self) -> Result<HashMap<PathBuf, LineStats>> {
        let base = self.diff_base()?;
        let mut args = vec!["diff", &base];
        if let Some(head) = self.diff_head() {
            args.push(head);
        }
        args.extend(["--numstat", "-z", "-M"]);
        let output = self.run_read_only(&args)?;
        Ok(parse_numstat(&output))
    }

    fn range_files(&self, spec: &RangeSpec) -> Result<Vec<ChangedFile>> {
        let base = self.diff_base()?;
        let raw = self.run_read_only(&["diff", "--raw", "-z", "-M", "--abbrev=40", &base, &spec.head])?;
        let stats = self.line_stats()?;
        Ok(parse_raw(&raw)
            .into_iter()
            .map(|entry| ChangedFile {
                stats: stats.get(&entry.path).copied().unwrap_or_default(),
                path: entry.path,
                kind: entry.kind,
                // Review marks live in ReviewService; every file starts unmarked
                // and the service reapplies what it remembers.
                stage: StageState::Unstaged,
                content_id: entry.content_id,
            })
            .collect())
    }

    fn worktree_files(&self) -> Result<Vec<ChangedFile>> {
        let status = self.run_read_only(&["status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
        let stats = self.line_stats()?;
        Ok(parse_porcelain_v2(&status)
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
                    content_id: None,
                }
            })
            .collect())
    }

    fn show(&self, rev: &str, path: &Path) -> Result<Option<Vec<u8>>> {
        let spec = format!("{rev}:{}", path.to_string_lossy());
        match self.run_read_only(&["show", &spec]) {
            Ok(content) => Ok(Some(content)),
            Err(_) => Ok(None),
        }
    }

    fn reject_in_range(&self) -> Result<()> {
        if let Some(spec) = self.mode.range() {
            bail!("{} is a read-only range review", spec.label());
        }
        Ok(())
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

    fn mode(&self) -> &RepoMode {
        &self.mode
    }

    fn branch_name(&self) -> Result<String> {
        if let Some(spec) = self.mode.range() {
            let range = format!("{}..{}", spec.base, spec.head);
            let commits = self
                .run_read_only(&["rev-list", "--count", &range])
                .map(|output| String::from_utf8_lossy(&output).trim().to_string())
                .unwrap_or_default();
            return Ok(match commits.as_str() {
                "" => spec.label(),
                "1" => format!("{} · 1 commit", spec.label()),
                count => format!("{} · {count} commits", spec.label()),
            });
        }
        if let Ok(output) = self.run_read_only(&["symbolic-ref", "--short", "-q", "HEAD"]) {
            return Ok(String::from_utf8_lossy(&output).trim().to_string());
        }
        let output = self.run_read_only(&["rev-parse", "--short", "HEAD"])?;
        Ok(format!("detached @ {}", String::from_utf8_lossy(&output).trim()))
    }

    fn review_head_id(&self) -> Option<String> {
        let spec = self.mode.range()?;
        self.resolve(&spec.head).ok()
    }

    fn refs(&self) -> Result<Vec<GitRef>> {
        let output = self.run_read_only(&[
            "for-each-ref",
            // creatordate rather than committerdate: annotated tags have no
            // committer, so sorting by that drops every tag to the bottom.
            "--sort=-creatordate",
            "--format=%(refname)\t%(refname:short)\t%(creatordate:relative)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?;
        Ok(parse_refs(&String::from_utf8_lossy(&output)))
    }

    fn changed_files(&self) -> Result<Vec<ChangedFile>> {
        match self.mode.range() {
            Some(spec) => self.range_files(spec),
            None => self.worktree_files(),
        }
    }

    fn source_files(&self, extension: &str) -> Result<Vec<PathBuf>> {
        // In range mode the symbol index has to resolve against the head ref, so
        // cmd-click lands on the code you are reading rather than your checkout.
        let output = match self.diff_head() {
            Some(head) => self.run_read_only(&["ls-tree", "-r", "--name-only", "-z", head])?,
            None => {
                let pattern = format!("*.{extension}");
                self.run_read_only(&[
                    "ls-files",
                    "-z",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "--",
                    &pattern,
                ])?
            }
        };
        let suffix = format!(".{extension}");
        let mut files: Vec<PathBuf> = output
            .split(|&byte| byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf8_lossy(entry).into_owned())
            .filter(|entry| entry.ends_with(&suffix))
            .map(PathBuf::from)
            .collect();
        files.sort();
        files.dedup();
        Ok(files)
    }

    fn head_content(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        self.show(&self.diff_base()?, path)
    }

    fn new_content(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        let Some(head) = self.diff_head() else {
            let absolute = self.root.join(path);
            // Not every entry git reports is a readable file: an untracked
            // nested repository arrives as `dir/`, and a symlink can point at a
            // directory. There is nothing to diff, and returning an error here
            // strands the view on "Loading…" and retries once a second forever.
            if std::fs::metadata(&absolute).is_ok_and(|meta| !meta.is_file()) {
                return Ok(None);
            }
            return match std::fs::read(&absolute) {
                Ok(content) => Ok(Some(content)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
            };
        };
        self.show(head, path)
    }

    fn worktree_stamp(&self, path: &Path) -> Option<FileStamp> {
        if self.mode.is_range() {
            return None;
        }
        let metadata = std::fs::metadata(self.root.join(path)).ok()?;
        Some(FileStamp {
            modified: metadata.modified().ok()?,
            len: metadata.len(),
        })
    }

    fn stage(&self, paths: &[&Path]) -> Result<()> {
        self.reject_in_range()?;
        self.run_with_paths(&["add", "-A"], paths)
    }

    fn unstage(&self, paths: &[&Path]) -> Result<()> {
        self.reject_in_range()?;
        if self.has_head() {
            self.run_with_paths(&["reset", "-q", "HEAD"], paths)
        } else {
            self.run_with_paths(&["rm", "-r", "-q", "--cached"], paths)
        }
    }

    fn stage_all(&self) -> Result<()> {
        self.reject_in_range()?;
        self.run(&["add", "-A"]).map(|_| ())
    }

    fn unstage_all(&self) -> Result<()> {
        self.reject_in_range()?;
        if self.has_head() {
            self.run(&["reset", "-q"]).map(|_| ())
        } else {
            self.run(&["rm", "-r", "-q", "--cached", "."]).map(|_| ())
        }
    }

    fn discard(&self, file: &ChangedFile) -> Result<()> {
        self.reject_in_range()?;
        match file.kind {
            ChangeKind::Untracked => std::fs::remove_file(self.root.join(&file.path))
                .with_context(|| format!("deleting {}", file.path.display())),
            _ => {
                let base = self.diff_base()?;
                let source = format!("--source={base}");
                self.run_with_paths(&["restore", &source, "--staged", "--worktree"], &file.all_paths())
            }
        }
    }

    fn commit(&self, message: &str) -> Result<String> {
        self.reject_in_range()?;
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

struct RawEntry {
    path: PathBuf,
    kind: ChangeKind,
    content_id: Option<String>,
}

/// Parses `git diff --raw -z` records:
/// `:<src mode> <dst mode> <src sha> <dst sha> <status>\0<path>\0`, with a
/// second path following for renames.
fn parse_raw(bytes: &[u8]) -> Vec<RawEntry> {
    let mut tokens = bytes
        .split(|&byte| byte == 0)
        .filter(|token| !token.is_empty())
        .map(|token| String::from_utf8_lossy(token).into_owned());
    let mut entries = Vec::new();
    while let Some(record) = tokens.next() {
        let Some(meta) = record.strip_prefix(':') else {
            continue;
        };
        let fields: Vec<&str> = meta.split(' ').collect();
        let [_src_mode, _dst_mode, src_sha, dst_sha, status] = fields.as_slice() else {
            continue;
        };
        let content_id = Some(format!("{src_sha}:{dst_sha}"));
        let Some(path) = tokens.next() else {
            break;
        };
        let letter = status.chars().next().unwrap_or('M');
        let kind = match letter {
            'A' => ChangeKind::Added,
            'D' => ChangeKind::Deleted,
            'R' => match tokens.next() {
                Some(destination) => {
                    entries.push(RawEntry {
                        path: PathBuf::from(destination),
                        kind: ChangeKind::Renamed { from: PathBuf::from(path) },
                        content_id,
                    });
                    continue;
                }
                None => break,
            },
            _ => ChangeKind::Modified,
        };
        entries.push(RawEntry {
            path: PathBuf::from(path),
            kind,
            content_id,
        });
    }
    entries
}

fn parse_refs(text: &str) -> Vec<GitRef> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let full = fields.next()?;
            let name = fields.next()?;
            let age = fields.next().unwrap_or_default();
            // `refs/remotes/<remote>/HEAD` is a symbolic alias, not somewhere to
            // diff from; it would show up as a bare remote name and confuse.
            if full.ends_with("/HEAD") {
                return None;
            }
            let kind = if full.starts_with("refs/heads/") {
                RefKind::Branch
            } else if full.starts_with("refs/remotes/") {
                RefKind::Remote
            } else if full.starts_with("refs/tags/") {
                RefKind::Tag
            } else {
                return None;
            };
            Some(GitRef {
                name: name.to_string(),
                kind,
                age: age.to_string(),
            })
        })
        .collect()
}

fn toplevel(start: &Path) -> Result<PathBuf> {
    let output = Command::new(GIT)
        .arg("-C")
        .arg(start)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("failed to run git")?;
    if !output.status.success() {
        bail!("{} is not inside a git repository", start.display());
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
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
mod worktree_tests {
    use super::*;

    /// A repo containing an untracked nested repository, which `git status`
    /// reports as a single directory entry rather than a file.
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("diffviewer-wt-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let run = |dir: &Path, args: &[&str]| {
            let out = Command::new(GIT).arg("-C").arg(dir).args(args).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&root, &["init", "-q", "-b", "main"]);
        run(&root, &["config", "user.email", "test@example.com"]);
        run(&root, &["config", "user.name", "Test"]);
        run(&root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "hi\n").unwrap();
        run(&root, &["add", "-A"]);
        run(&root, &["commit", "-qm", "base"]);
        run(&root.join("nested"), &["init", "-q"]);
        std::fs::write(root.join("nested/f.txt"), "x\n").unwrap();
        root
    }

    #[test]
    fn a_directory_entry_has_no_content_rather_than_erroring() {
        // Erroring here stranded the diff view on "Loading…" forever, retrying
        // once a second, because the failure was reported but never recorded.
        let root = fixture("nested-repo");
        let repo = GitCli::discover(&root).unwrap();
        let files = repo.changed_files().unwrap();
        let entry = files
            .iter()
            .find(|file| file.path.to_string_lossy().starts_with("nested"))
            .expect("git reports the nested repo as an entry");
        assert!(repo.new_content(&entry.path).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ordinary_files_still_read_normally() {
        let root = fixture("plain-file");
        let repo = GitCli::discover(&root).unwrap();
        std::fs::write(root.join("a.txt"), "changed\n").unwrap();
        let content = repo.new_content(Path::new("a.txt")).unwrap().unwrap();
        assert_eq!(String::from_utf8_lossy(&content), "changed\n");
        assert!(repo.new_content(Path::new("missing.txt")).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;
    use crate::application::repository::RangeSpec;

    /// Builds a repo with a `feature` branch forked from `main`, plus a later
    /// commit on `main` that the review must not show.
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("diffviewer-test-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| {
            let status = Command::new(GIT).arg("-C").arg(&root).args(args).output().unwrap();
            assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
        };
        let write = |path: &str, body: &str| std::fs::write(root.join(path), body).unwrap();
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Test"]);
        // Local config wins over a global commit.gpgsign=true, which would
        // otherwise make these fixtures fail on signing-enabled machines.
        git(&["config", "commit.gpgsign", "false"]);
        write("keep.py", "x = 1\n");
        write("gone.py", "y = 2\n");
        write("old.py", "def helper():\n    return 1\n");
        git(&["add", "-A"]);
        git(&["commit", "-qm", "base"]);

        git(&["checkout", "-q", "-b", "feature"]);
        write("keep.py", "x = 1\nx = 2\n");
        std::fs::remove_file(root.join("gone.py")).unwrap();
        std::fs::rename(root.join("old.py"), root.join("new.py")).unwrap();
        write("added.py", "z = 3\n");
        git(&["add", "-A"]);
        git(&["commit", "-qm", "work"]);

        // A commit on main *after* the fork. Merge-base semantics must hide it.
        git(&["checkout", "-q", "main"]);
        write("unrelated.py", "w = 4\n");
        git(&["add", "-A"]);
        git(&["commit", "-qm", "later on main"]);
        git(&["checkout", "-q", "feature"]);
        root
    }

    fn spec() -> RangeSpec {
        RangeSpec { base: "main".into(), head: "feature".into() }
    }

    #[test]
    fn lists_only_the_branch_own_changes() {
        let root = fixture("own-changes");
        let repo = GitCli::discover_range(&root, spec()).unwrap();
        let files = repo.changed_files().unwrap();
        let names: Vec<String> = files.iter().map(|f| f.path.display().to_string()).collect();

        // `unrelated.py` landed on main after the fork; merge-base semantics excludes it.
        assert!(!names.contains(&"unrelated.py".to_string()), "got {names:?}");
        assert!(names.contains(&"keep.py".to_string()), "got {names:?}");
        assert!(names.contains(&"added.py".to_string()), "got {names:?}");
        assert!(names.contains(&"gone.py".to_string()), "got {names:?}");
        assert!(names.contains(&"new.py".to_string()), "got {names:?}");

        let kind = |path: &str| files.iter().find(|f| f.path == PathBuf::from(path)).map(|f| f.kind.clone());
        assert_eq!(kind("added.py"), Some(ChangeKind::Added));
        assert_eq!(kind("gone.py"), Some(ChangeKind::Deleted));
        assert_eq!(kind("keep.py"), Some(ChangeKind::Modified));
        assert_eq!(kind("new.py"), Some(ChangeKind::Renamed { from: PathBuf::from("old.py") }));

        let keep = files.iter().find(|f| f.path == PathBuf::from("keep.py")).unwrap();
        assert_eq!(keep.stats, LineStats { added: 1, removed: 0 });
        assert!(keep.content_id.is_some(), "modified files carry a content id to key review marks on");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_both_sides_from_commits_not_the_worktree() {
        let root = fixture("sides");
        let repo = GitCli::discover_range(&root, spec()).unwrap();
        // Dirty the worktree; a committed review must ignore it entirely.
        std::fs::write(root.join("keep.py"), "GARBAGE\n").unwrap();

        let old = repo.head_content(Path::new("keep.py")).unwrap().unwrap();
        let new = repo.new_content(Path::new("keep.py")).unwrap().unwrap();
        assert_eq!(String::from_utf8_lossy(&old), "x = 1\n");
        assert_eq!(String::from_utf8_lossy(&new), "x = 1\nx = 2\n");
        assert_eq!(repo.worktree_stamp(Path::new("keep.py")), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn source_files_come_from_the_head_ref() {
        let root = fixture("sources");
        let repo = GitCli::discover_range(&root, spec()).unwrap();
        let files = repo.source_files("py").unwrap();
        assert!(files.contains(&PathBuf::from("new.py")), "got {files:?}");
        assert!(!files.contains(&PathBuf::from("old.py")), "renamed away: {files:?}");
        assert!(!files.contains(&PathBuf::from("unrelated.py")), "only on main: {files:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mutations_are_refused() {
        let root = fixture("read-only");
        let repo = GitCli::discover_range(&root, spec()).unwrap();
        assert!(repo.stage_all().is_err());
        assert!(repo.unstage_all().is_err());
        assert!(repo.commit("nope").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bad_revision_fails_at_open() {
        let root = fixture("bad-rev");
        let spec = RangeSpec { base: "nope-not-a-branch".into(), head: "feature".into() };
        assert!(GitCli::discover_range(&root, spec).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn branch_name_reports_the_range_and_commit_count() {
        let root = fixture("label");
        let repo = GitCli::discover_range(&root, spec()).unwrap();
        assert_eq!(repo.branch_name().unwrap(), "main…feature · 1 commit");
        let _ = std::fs::remove_dir_all(&root);
    }
}

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
    fn parses_raw_entries_with_kinds_and_blobs() {
        let raw = b":100644 100644 1111111 2222222 M\0src/a.py\0:000000 100644 0000000 3333333 A\0src/new.py\0:100644 000000 4444444 0000000 D\0src/gone.py\0:100644 100644 5555555 6666666 R100\0src/old.py\0src/moved.py\0";
        let entries = parse_raw(raw);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].kind, ChangeKind::Modified);
        // Both sides are captured: the merge base moving must invalidate a mark.
        assert_eq!(entries[0].content_id.as_deref(), Some("1111111:2222222"));
        assert_eq!(entries[1].kind, ChangeKind::Added);
        assert_eq!(entries[2].kind, ChangeKind::Deleted);
        assert_eq!(entries[2].content_id.as_deref(), Some("4444444:0000000"));
        assert_eq!(entries[3].path, PathBuf::from("src/moved.py"));
        assert_eq!(entries[3].kind, ChangeKind::Renamed { from: PathBuf::from("src/old.py") });
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


