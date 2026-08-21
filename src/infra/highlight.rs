use std::ops::Range;
use std::path::Path;

use syntect::easy::HighlightLines;
use syntect::highlighting::{Style, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub color: Rgb,
}

pub struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
}

impl Highlighter {
    pub fn new() -> Self {
        let mut themes = ThemeSet::load_defaults();
        let theme = themes.themes.remove(THEME_NAME).unwrap_or_default();
        Self {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            theme,
        }
    }

    pub fn highlight(&self, path: &Path, text: &str) -> Vec<Vec<Span>> {
        let syntax = self.syntax_for(path, text);
        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        LinesWithEndings::from(text)
            .map(|line| {
                let visible_len = line.trim_end_matches(['\n', '\r']).len();
                match highlighter.highlight_line(line, &self.syntaxes) {
                    Ok(regions) => to_spans(&regions, visible_len),
                    Err(_) => Vec::new(),
                }
            })
            .collect()
    }

    fn syntax_for(&self, path: &Path, text: &str) -> &SyntaxReference {
        let by_extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| self.syntaxes.find_syntax_by_extension(ext));
        let by_file_name = || {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| self.syntaxes.find_syntax_by_extension(name))
        };
        let by_first_line = || {
            text.lines()
                .next()
                .and_then(|line| self.syntaxes.find_syntax_by_first_line(line))
        };
        by_extension
            .or_else(by_file_name)
            .or_else(by_first_line)
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text())
    }
}

fn to_spans(regions: &[(Style, &str)], visible_len: usize) -> Vec<Span> {
    let mut spans = Vec::with_capacity(regions.len());
    let mut start = 0;
    for (style, text) in regions {
        let end = (start + text.len()).min(visible_len);
        if end > start {
            spans.push(Span {
                range: start..end,
                color: Rgb {
                    r: style.foreground.r,
                    g: style.foreground.g,
                    b: style.foreground.b,
                },
            });
        }
        start += text.len();
    }
    spans
}

const THEME_NAME: &str = "base16-ocean.dark";
