use std::path::Path;

use super::change::ChangedFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tier {
    pub name: String,
    pub markers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportancePolicy {
    tiers: Vec<Tier>,
    test_markers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    pub tier: Option<usize>,
    pub is_test: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileGroup {
    pub title: String,
    pub is_tests: bool,
    pub files: Vec<ChangedFile>,
}

impl ImportancePolicy {
    pub fn new(tiers: Vec<Tier>, test_markers: Vec<String>) -> Self {
        let tiers = tiers
            .into_iter()
            .map(|tier| Tier {
                name: tier.name,
                markers: lowercase_all(tier.markers),
            })
            .collect();
        Self {
            tiers,
            test_markers: lowercase_all(test_markers),
        }
    }

    pub fn classify(&self, path: &Path) -> Classification {
        let components: Vec<String> = path
            .components()
            .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
            .collect();
        let (file_name, directories) = match components.split_last() {
            Some((name, dirs)) => (name.as_str(), dirs),
            None => ("", &[][..]),
        };
        let tier = directories
            .iter()
            .rev()
            .find_map(|dir| self.tier_for_directory(dir));
        let is_test = directories.iter().any(|dir| self.test_markers.contains(dir))
            || is_test_file_name(file_name);
        Classification { tier, is_test }
    }

    pub fn group(&self, files: Vec<ChangedFile>) -> Vec<FileGroup> {
        let mut per_tier: Vec<Vec<ChangedFile>> = self.tiers.iter().map(|_| Vec::new()).collect();
        let mut other = Vec::new();
        let mut tests = Vec::new();
        for file in files {
            let classification = self.classify(&file.path);
            if classification.is_test {
                tests.push(file);
                continue;
            }
            match classification.tier {
                Some(index) => per_tier[index].push(file),
                None => other.push(file),
            }
        }
        let mut groups = Vec::new();
        for (tier, files) in self.tiers.iter().zip(per_tier) {
            push_group(&mut groups, tier.name.clone(), false, files);
        }
        push_group(&mut groups, OTHER_GROUP.to_string(), false, other);
        push_group(&mut groups, TESTS_GROUP.to_string(), true, tests);
        groups
    }

    fn tier_for_directory(&self, dir: &str) -> Option<usize> {
        self.tiers
            .iter()
            .position(|tier| tier.markers.iter().any(|marker| marker == dir))
    }
}

fn push_group(groups: &mut Vec<FileGroup>, title: String, is_tests: bool, mut files: Vec<ChangedFile>) {
    if files.is_empty() {
        return;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    groups.push(FileGroup { title, is_tests, files });
}

fn is_test_file_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    stem.starts_with("test_")
        || stem.ends_with("_test")
        || stem == "conftest"
        || name.contains(".test.")
        || name.contains(".spec.")
}

fn lowercase_all(values: Vec<String>) -> Vec<String> {
    values.into_iter().map(|value| value.to_lowercase()).collect()
}

const OTHER_GROUP: &str = "Other";
const TESTS_GROUP: &str = "Tests";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::change::{ChangeKind, LineStats, StageState};
    use std::path::PathBuf;

    fn policy() -> ImportancePolicy {
        ImportancePolicy::new(
            vec![
                Tier { name: "Domain".into(), markers: vec!["domain".into()] },
                Tier { name: "Use cases".into(), markers: vec!["use_cases".into()] },
                Tier { name: "Application".into(), markers: vec!["application".into()] },
                Tier { name: "Infrastructure".into(), markers: vec!["infrastructure".into()] },
            ],
            vec!["tests".into()],
        )
    }

    fn file(path: &str) -> ChangedFile {
        ChangedFile {
            path: PathBuf::from(path),
            kind: ChangeKind::Modified,
            stage: StageState::Unstaged,
            stats: LineStats::default(),
            content_id: None,
        }
    }

    #[test]
    fn innermost_layer_directory_wins() {
        let classification = policy().classify(Path::new("src/app/domain/models.py"));
        assert_eq!(classification.tier, Some(0));
        assert!(!classification.is_test);
    }

    #[test]
    fn tests_are_detected_by_directory_and_name() {
        assert!(policy().classify(Path::new("src/x/use_cases/tests/foo.py")).is_test);
        assert!(policy().classify(Path::new("src/x/use_cases/test_foo.py")).is_test);
        assert!(policy().classify(Path::new("web/foo.spec.ts")).is_test);
        assert!(!policy().classify(Path::new("src/x/use_cases/testing_tools.py")).is_test);
    }

    #[test]
    fn groups_follow_tier_order_with_tests_last() {
        let groups = policy().group(vec![
            file("src/a/infrastructure/db.py"),
            file("src/a/domain/tests/test_x.py"),
            file("README.md"),
            file("src/a/use_cases/service.py"),
            file("src/a/domain/model.py"),
        ]);
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, vec!["Domain", "Use cases", "Infrastructure", "Other", "Tests"]);
        assert!(groups.last().unwrap().is_tests);
    }
}
