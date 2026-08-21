use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Class,
    Function,
    Method,
    Variable,
    Module,
}

impl SymbolKind {
    pub fn label(&self) -> &'static str {
        match self {
            SymbolKind::Class => "class",
            SymbolKind::Function => "def",
            SymbolKind::Method => "method",
            SymbolKind::Variable => "var",
            SymbolKind::Module => "module",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub path: PathBuf,
    pub line: usize,
    pub container: Option<String>,
}

impl Symbol {
    pub fn qualified_name(&self) -> String {
        match &self.container {
            Some(container) => format!("{container}.{}", self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    From {
        level: usize,
        module: String,
        name: String,
        alias: Option<String>,
    },
    Module {
        module: String,
        alias: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedModule {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
}

#[derive(Debug, Clone, Copy)]
pub struct DefinitionQuery<'a> {
    pub file: &'a Path,
    pub word: &'a str,
    pub attribute_access: bool,
}

#[derive(Debug, Default)]
pub struct SymbolIndex {
    modules: HashMap<PathBuf, ParsedModule>,
    by_name: HashMap<String, Vec<Symbol>>,
}

impl SymbolIndex {
    pub fn insert(&mut self, path: PathBuf, module: ParsedModule) {
        self.remove(&path);
        for symbol in &module.symbols {
            self.by_name.entry(symbol.name.clone()).or_default().push(symbol.clone());
        }
        self.modules.insert(path, module);
    }

    pub fn remove(&mut self, path: &Path) {
        let Some(previous) = self.modules.remove(path) else {
            return;
        };
        for symbol in previous.symbols {
            if let Some(entries) = self.by_name.get_mut(&symbol.name) {
                entries.retain(|entry| entry.path != path);
            }
        }
    }

    pub fn file_count(&self) -> usize {
        self.modules.len()
    }

    pub fn resolve(&self, query: DefinitionQuery) -> Vec<Symbol> {
        let mut found = Vec::new();
        if !query.attribute_access {
            found.extend(self.resolve_in_scope(query.file, query.word));
        }
        if found.is_empty() {
            found = self.resolve_globally(query);
        }
        dedup_in_place(&mut found);
        found
    }

    fn resolve_in_scope(&self, file: &Path, word: &str) -> Vec<Symbol> {
        let Some(module) = self.modules.get(file) else {
            return Vec::new();
        };
        let mut found: Vec<Symbol> = module
            .symbols
            .iter()
            .filter(|symbol| symbol.name == word && symbol.kind != SymbolKind::Method)
            .cloned()
            .collect();
        for import in &module.imports {
            match import {
                Import::From { level, module: module_name, name, alias } if bound_name(name, alias) == word => {
                    for target in self.resolve_module(file, *level, module_name) {
                        found.extend(self.exported(&target, name, 0));
                    }
                    let submodule = join_module(module_name, name);
                    found.extend(self.resolve_module(file, *level, &submodule).into_iter().map(module_symbol));
                }
                Import::Module { module: module_name, alias } if bound_name(module_name, alias) == word => {
                    found.extend(self.resolve_module(file, 0, module_name).into_iter().map(module_symbol));
                }
                _ => {}
            }
        }
        found
    }

    fn resolve_globally(&self, query: DefinitionQuery) -> Vec<Symbol> {
        let mut found = self.by_name.get(query.word).cloned().unwrap_or_default();
        let rank = |symbol: &Symbol| -> usize {
            match (query.attribute_access, &symbol.kind) {
                (true, SymbolKind::Method) => 0,
                (true, SymbolKind::Function) => 1,
                (true, SymbolKind::Variable) => 2,
                (true, _) => 3,
                (false, SymbolKind::Class) => 0,
                (false, SymbolKind::Function) => 1,
                (false, SymbolKind::Variable) => 2,
                (false, _) => 3,
            }
        };
        found.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then_with(|| proximity(query.file, &b.path).cmp(&proximity(query.file, &a.path)))
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.line.cmp(&b.line))
        });
        found
    }

    fn exported(&self, file: &Path, name: &str, depth: usize) -> Vec<Symbol> {
        let Some(module) = self.modules.get(file) else {
            return Vec::new();
        };
        let direct: Vec<Symbol> = module
            .symbols
            .iter()
            .filter(|symbol| symbol.name == name && symbol.kind != SymbolKind::Method)
            .cloned()
            .collect();
        if !direct.is_empty() || depth >= MAX_REEXPORT_DEPTH {
            return direct;
        }
        for import in &module.imports {
            let Import::From { level, module: module_name, name: imported, alias } = import else {
                continue;
            };
            if bound_name(imported, alias) != name {
                continue;
            }
            for target in self.resolve_module(file, *level, module_name) {
                let resolved = self.exported(&target, imported, depth + 1);
                if !resolved.is_empty() {
                    return resolved;
                }
            }
        }
        Vec::new()
    }

    fn resolve_module(&self, from: &Path, level: usize, module: &str) -> Vec<PathBuf> {
        let relative: PathBuf = module.split('.').filter(|part| !part.is_empty()).collect();
        if level > 0 {
            let mut base = from.parent().unwrap_or(Path::new("")).to_path_buf();
            for _ in 1..level {
                base.pop();
            }
            let candidates = if relative.as_os_str().is_empty() {
                vec![base.join(INIT_FILE)]
            } else {
                let stem = base.join(&relative);
                vec![stem.with_extension(PYTHON_EXTENSION), stem.join(INIT_FILE)]
            };
            return candidates
                .into_iter()
                .filter(|candidate| self.modules.contains_key(candidate))
                .collect();
        }
        if relative.as_os_str().is_empty() {
            return Vec::new();
        }
        let file_suffix = relative.with_extension(PYTHON_EXTENSION);
        let package_suffix = relative.join(INIT_FILE);
        let mut matches: Vec<PathBuf> = self
            .modules
            .keys()
            .filter(|path| path.ends_with(&file_suffix) || path.ends_with(&package_suffix))
            .cloned()
            .collect();
        matches.sort();
        matches
    }
}

fn bound_name<'a>(name: &'a str, alias: &'a Option<String>) -> &'a str {
    alias.as_deref().unwrap_or(name)
}

fn join_module(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.to_string()
    } else {
        format!("{module}.{name}")
    }
}

fn module_symbol(path: PathBuf) -> Symbol {
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    Symbol {
        name,
        kind: SymbolKind::Module,
        path,
        line: 1,
        container: None,
    }
}

fn proximity(a: &Path, b: &Path) -> usize {
    let dir_a = a.parent().unwrap_or(Path::new(""));
    let dir_b = b.parent().unwrap_or(Path::new(""));
    dir_a
        .components()
        .zip(dir_b.components())
        .take_while(|(x, y)| x == y)
        .count()
}

fn dedup_in_place(symbols: &mut Vec<Symbol>) {
    let mut seen = std::collections::HashSet::new();
    symbols.retain(|symbol| seen.insert((symbol.path.clone(), symbol.line)));
}

const MAX_REEXPORT_DEPTH: usize = 3;
const INIT_FILE: &str = "__init__.py";
const PYTHON_EXTENSION: &str = "py";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::python::parse_python;

    fn index() -> SymbolIndex {
        let mut index = SymbolIndex::default();
        let files = [
            ("src/location/domain/models.py", "class LocationId:\n    pass\n\nclass Site:\n    def name(self) -> str:\n        return ''\n"),
            ("src/location/domain/__init__.py", "from .models import LocationId\n"),
            ("src/voice/use_cases/service.py", "class VoiceService:\n    def get_traces(self, x):\n        return x\n"),
            (
                "src/voice/application/routes.py",
                "from location.domain import LocationId\nfrom ..use_cases import service as svc\nfrom voice.use_cases.service import VoiceService\n\nLIMIT = 3\n\ndef handler(service: VoiceService) -> LocationId:\n    return service.get_traces(LIMIT)\n",
            ),
            ("src/voice/use_cases/__init__.py", ""),
        ];
        for (path, source) in files {
            let path = PathBuf::from(path);
            index.insert(path.clone(), parse_python(&path, source));
        }
        index
    }

    fn query<'a>(word: &'a str, attribute_access: bool) -> DefinitionQuery<'a> {
        DefinitionQuery {
            file: Path::new("src/voice/application/routes.py"),
            word,
            attribute_access,
        }
    }

    #[test]
    fn follows_import_through_package_reexport() {
        let found = index().resolve(query("LocationId", false));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, PathBuf::from("src/location/domain/models.py"));
        assert_eq!(found[0].line, 1);
    }

    #[test]
    fn resolves_local_symbol_before_anything_else() {
        let found = index().resolve(query("LIMIT", false));
        assert_eq!(found[0].path, PathBuf::from("src/voice/application/routes.py"));
        assert_eq!(found[0].kind, SymbolKind::Variable);
    }

    #[test]
    fn resolves_direct_import_and_relative_module_alias() {
        let service = index().resolve(query("VoiceService", false));
        assert_eq!(service[0].path, PathBuf::from("src/voice/use_cases/service.py"));
        let module = index().resolve(query("svc", false));
        assert_eq!(module[0].kind, SymbolKind::Module);
        assert_eq!(module[0].path, PathBuf::from("src/voice/use_cases/service.py"));
    }

    #[test]
    fn attribute_access_prefers_methods_by_name() {
        let found = index().resolve(query("get_traces", true));
        assert_eq!(found[0].kind, SymbolKind::Method);
        assert_eq!(found[0].container.as_deref(), Some("VoiceService"));
    }
}
