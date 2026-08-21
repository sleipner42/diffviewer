use std::path::Path;

use super::symbols::{Import, ParsedModule, Symbol, SymbolKind};

pub fn parse_python(path: &Path, source: &str) -> ParsedModule {
    let mut parsed = ParsedModule::default();
    let mut class_stack: Vec<(usize, String)> = Vec::new();
    let mut in_string = false;
    let mut pending_import: Option<String> = None;
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        if let Some(accumulated) = pending_import.as_mut() {
            accumulated.push(' ');
            accumulated.push_str(raw.trim());
            if raw.contains(')') {
                parse_import(accumulated, &mut parsed.imports);
                pending_import = None;
            }
            continue;
        }
        let triple_quotes = count_triple_quotes(raw);
        if in_string {
            if triple_quotes % 2 == 1 {
                in_string = false;
            }
            continue;
        }
        let trimmed = raw.trim_start();
        let indent = raw.len() - trimmed.len();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        while class_stack.last().is_some_and(|(class_indent, _)| *class_indent >= indent) {
            class_stack.pop();
        }
        if let Some(rest) = trimmed.strip_prefix("class ") {
            let name = identifier_prefix(rest);
            if !name.is_empty() {
                let container = class_stack.last().map(|(_, class_name)| class_name.clone());
                parsed.symbols.push(symbol(path, name, SymbolKind::Class, line, container));
                class_stack.push((indent, name.to_string()));
            }
        } else if let Some(rest) = strip_def(trimmed) {
            let name = identifier_prefix(rest);
            if !name.is_empty() {
                let (kind, container) = match class_stack.last() {
                    Some((_, class_name)) => (SymbolKind::Method, Some(class_name.clone())),
                    None => (SymbolKind::Function, None),
                };
                parsed.symbols.push(symbol(path, name, kind, line, container));
            }
        } else if trimmed.starts_with("from ") || trimmed.starts_with("import ") {
            if trimmed.contains('(') && !trimmed.contains(')') {
                pending_import = Some(trimmed.to_string());
            } else {
                parse_import(trimmed, &mut parsed.imports);
            }
        } else if indent == 0 {
            if let Some(name) = assignment_target(trimmed) {
                parsed.symbols.push(symbol(path, name, SymbolKind::Variable, line, None));
            }
        }
        if triple_quotes % 2 == 1 {
            in_string = true;
        }
    }
    parsed
}

fn symbol(path: &Path, name: &str, kind: SymbolKind, line: usize, container: Option<String>) -> Symbol {
    Symbol {
        name: name.to_string(),
        kind,
        path: path.to_path_buf(),
        line,
        container,
    }
}

fn strip_def(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix("def ")
        .or_else(|| trimmed.strip_prefix("async def "))
}

fn parse_import(text: &str, imports: &mut Vec<Import>) {
    let text = text.trim().trim_end_matches(';');
    if let Some(rest) = text.strip_prefix("from ") {
        let Some((module_part, names_part)) = rest.split_once(" import ") else {
            return;
        };
        let module_part = module_part.trim();
        let level = module_part.chars().take_while(|&c| c == '.').count();
        let module = module_part[level..].to_string();
        let names = names_part.trim().trim_start_matches('(').trim_end_matches(')');
        for entry in names.split(',') {
            let Some((name, alias)) = split_alias(entry) else {
                continue;
            };
            if name == "*" {
                continue;
            }
            imports.push(Import::From {
                level,
                module: module.clone(),
                name,
                alias,
            });
        }
    } else if let Some(rest) = text.strip_prefix("import ") {
        for entry in rest.split(',') {
            if let Some((module, alias)) = split_alias(entry) {
                imports.push(Import::Module { module, alias });
            }
        }
    }
}

fn split_alias(entry: &str) -> Option<(String, Option<String>)> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    match entry.split_once(" as ") {
        Some((name, alias)) => Some((name.trim().to_string(), Some(alias.trim().to_string()))),
        None => Some((entry.to_string(), None)),
    }
}

fn assignment_target(trimmed: &str) -> Option<&str> {
    let name = identifier_prefix(trimmed);
    if name.is_empty() {
        return None;
    }
    let rest = trimmed[name.len()..].trim_start();
    let assignment = match rest.strip_prefix(':') {
        Some(annotated) => match annotated.find('=') {
            Some(index) if !annotated[index + 1..].starts_with('=') => &annotated[index..],
            _ => return None,
        },
        None => rest,
    };
    if assignment.starts_with('=') && !assignment.starts_with("==") {
        Some(name)
    } else {
        None
    }
}

fn identifier_prefix(text: &str) -> &str {
    let mut end = 0;
    for (offset, character) in text.char_indices() {
        let valid = character == '_' || character.is_alphanumeric();
        let valid_start = offset > 0 || !character.is_numeric();
        if !(valid && valid_start) {
            break;
        }
        end = offset + character.len_utf8();
    }
    &text[..end]
}

fn count_triple_quotes(line: &str) -> usize {
    line.matches("\"\"\"").count() + line.matches("'''").count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SOURCE: &str = r#"from dataclasses import dataclass
from location.domain.models import (
    LocationId,
    Site as SiteModel,
)
import os.path as osp
from . import helpers

MAX_TRACE_LIMIT = 100
ValidatedSaoId: TypeAlias = Annotated[str, Depends(check)]


@dataclass(frozen=True)
class CallerContext:
    """Docstring with def inside() that must be ignored."""

    caller_id: str

    @classmethod
    def from_caller_id(cls, caller_id: str) -> CallerContext:
        return cls(caller_id=caller_id)


async def _window_from_query(limit: int) -> int:
    if limit == 3:
        return limit
    return limit
"#;

    fn parsed() -> ParsedModule {
        parse_python(&PathBuf::from("src/a.py"), SOURCE)
    }

    #[test]
    fn finds_classes_functions_methods_and_constants() {
        let names: Vec<(String, SymbolKind, usize)> = parsed()
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.kind.clone(), s.line))
            .collect();
        assert_eq!(
            names,
            vec![
                ("MAX_TRACE_LIMIT".into(), SymbolKind::Variable, 9),
                ("ValidatedSaoId".into(), SymbolKind::Variable, 10),
                ("CallerContext".into(), SymbolKind::Class, 14),
                ("from_caller_id".into(), SymbolKind::Method, 20),
                ("_window_from_query".into(), SymbolKind::Function, 24),
            ]
        );
        assert_eq!(parsed().symbols[3].container.as_deref(), Some("CallerContext"));
    }

    #[test]
    fn parses_imports_including_multiline_and_relative() {
        let imports = parsed().imports;
        assert!(imports.contains(&Import::From { level: 0, module: "dataclasses".into(), name: "dataclass".into(), alias: None }));
        assert!(imports.contains(&Import::From { level: 0, module: "location.domain.models".into(), name: "LocationId".into(), alias: None }));
        assert!(imports.contains(&Import::From { level: 0, module: "location.domain.models".into(), name: "Site".into(), alias: Some("SiteModel".into()) }));
        assert!(imports.contains(&Import::Module { module: "os.path".into(), alias: Some("osp".into()) }));
        assert!(imports.contains(&Import::From { level: 1, module: String::new(), name: "helpers".into(), alias: None }));
    }
}
