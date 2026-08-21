use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn open_in_editor(template: &str, path: &Path, line: usize) -> Result<()> {
    let mut parts = template.split_whitespace().map(|part| {
        part.replace(PATH_PLACEHOLDER, &path.to_string_lossy())
            .replace(LINE_PLACEHOLDER, &line.to_string())
    });
    let Some(program) = parts.next() else {
        bail!("editor command is empty");
    };
    Command::new(&program)
        .args(parts)
        .spawn()
        .with_context(|| format!("launching {program}"))?;
    Ok(())
}

pub fn editor_display_name(template: &str) -> String {
    let program = template.split_whitespace().next().unwrap_or("editor");
    let name = Path::new(program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string());
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => name,
    }
}

const PATH_PLACEHOLDER: &str = "{path}";
const LINE_PLACEHOLDER: &str = "{line}";
