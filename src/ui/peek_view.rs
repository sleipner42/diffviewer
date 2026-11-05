use std::path::PathBuf;

use egui::{Align, Button, Color32, CursorIcon, Layout, RichText, ScrollArea, Sense, TextStyle, Ui, vec2};

use super::code_line::{CodeLine, GUTTER_PADDING, LineStyle, PointerState, ROW_PADDING_Y, TEXT_PADDING, WordHit, paint_code_line};
use super::keymap;
use super::theme::Theme;
use crate::application::review_service::PeekSource;
use crate::domain::symbols::Symbol;
use crate::infra::highlight::{Highlighter, Span};

pub struct LoadedSource {
    pub path: PathBuf,
    pub lines: Vec<String>,
    pub spans: Vec<Vec<Span>>,
    pub target_line: usize,
    pub max_chars: usize,
}

impl LoadedSource {
    pub fn build(source: PeekSource, highlighter: &Highlighter) -> Self {
        let spans = highlighter.highlight(&source.path, &source.text);
        let lines: Vec<String> = source
            .text
            .lines()
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect();
        let max_chars = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0);
        Self {
            path: source.path,
            lines,
            spans,
            target_line: source.line,
            max_chars,
        }
    }
}

pub struct PeekFrame {
    candidates: Vec<Symbol>,
    chosen: usize,
}

pub struct PeekState {
    pub candidates: Vec<Symbol>,
    pub chosen: usize,
    pub source: Option<LoadedSource>,
    history: Vec<PeekFrame>,
    scroll_pending: bool,
}

impl PeekState {
    pub fn new(candidates: Vec<Symbol>) -> Self {
        Self {
            candidates,
            chosen: 0,
            source: None,
            history: Vec::new(),
            scroll_pending: true,
        }
    }

    pub fn symbol(&self) -> Option<&Symbol> {
        self.candidates.get(self.chosen)
    }

    pub fn choose(&mut self, index: usize) {
        if index < self.candidates.len() && index != self.chosen {
            self.chosen = index;
            self.source = None;
            self.scroll_pending = true;
        }
    }

    pub fn navigate(&mut self, candidates: Vec<Symbol>) {
        let previous = PeekFrame {
            candidates: std::mem::take(&mut self.candidates),
            chosen: self.chosen,
        };
        self.history.push(previous);
        self.candidates = candidates;
        self.chosen = 0;
        self.source = None;
        self.scroll_pending = true;
    }

    pub fn back(&mut self) -> bool {
        let Some(frame) = self.history.pop() else {
            return false;
        };
        self.candidates = frame.candidates;
        self.chosen = frame.chosen;
        self.source = None;
        self.scroll_pending = true;
        true
    }

    pub fn has_history(&self) -> bool {
        !self.history.is_empty()
    }
}

pub enum PeekAction {
    Close,
    Back,
    Choose(usize),
    Navigate(WordHit),
    OpenInEditor,
}

pub fn show(ui: &mut Ui, state: &mut PeekState, editor_name: &str, theme: &Theme) -> Option<PeekAction> {
    let mut action = None;
    header(ui, state, editor_name, theme, &mut action);
    if state.candidates.len() > 1 {
        candidate_row(ui, state, theme, &mut action);
    }
    match &state.source {
        Some(source) => {
            if let Some(hit) = body(ui, source, &mut state.scroll_pending, theme) {
                action = Some(PeekAction::Navigate(hit));
            }
        }
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Loading…").color(theme.text_dim));
            });
        }
    }
    action
}

fn header(ui: &mut Ui, state: &PeekState, editor_name: &str, theme: &Theme, action: &mut Option<PeekAction>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        if let Some(symbol) = state.symbol() {
            ui.label(RichText::new(symbol.kind.label()).small().color(theme.accent));
            ui.label(RichText::new(symbol.qualified_name()).text_style(TextStyle::Monospace).strong());
            ui.label(
                RichText::new(format!("{}:{}", symbol.path.display(), symbol.line))
                    .small()
                    .color(theme.text_dim),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            if ui.button("Close  esc").clicked() {
                *action = Some(PeekAction::Close);
            }
            if ui.button(format!("Open in {editor_name}")).clicked() {
                *action = Some(PeekAction::OpenInEditor);
            }
            if ui.add_enabled(state.has_history(), Button::new(format!("Back  {}", keymap::key_label("[")))).clicked() {
                *action = Some(PeekAction::Back);
            }
        });
    });
    ui.add_space(4.0);
}

fn candidate_row(ui: &mut Ui, state: &PeekState, theme: &Theme, action: &mut Option<PeekAction>) {
    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("{} definitions:", state.candidates.len()))
                .small()
                .color(theme.text_dim),
        );
        for (index, symbol) in state.candidates.iter().enumerate().take(MAX_CANDIDATES_SHOWN) {
            let file_name = symbol
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let label = format!("{} {}:{}", symbol.kind.label(), file_name, symbol.line);
            if ui.selectable_label(index == state.chosen, RichText::new(label).small()).clicked() {
                *action = Some(PeekAction::Choose(index));
            }
        }
        if state.candidates.len() > MAX_CANDIDATES_SHOWN {
            ui.label(
                RichText::new(format!("+{} more", state.candidates.len() - MAX_CANDIDATES_SHOWN))
                    .small()
                    .color(theme.text_faint),
            );
        }
    });
    ui.add_space(4.0);
}

fn body(ui: &mut Ui, source: &LoadedSource, scroll_pending: &mut bool, theme: &Theme) -> Option<WordHit> {
    let font = TextStyle::Monospace.resolve(ui.style());
    let char_width = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, 'M'));
    let row_height = ui.text_style_height(&TextStyle::Monospace) + ROW_PADDING_Y * 2.0;
    let digits = source.lines.len().max(1).to_string().len() as f32;
    let gutter_width = (digits + 1.0) * char_width + GUTTER_PADDING * 2.0;
    let total_width =
        (gutter_width + source.max_chars as f32 * char_width + TEXT_PADDING * 2.0).max(ui.available_width());
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut scroll = ScrollArea::both()
        .id_salt(("peek", &source.path, source.target_line))
        .auto_shrink([false, false]);
    if *scroll_pending {
        *scroll_pending = false;
        let offset = (source.target_line.saturating_sub(1) as f32 * row_height - SCROLL_MARGIN).max(0.0);
        scroll = scroll.vertical_scroll_offset(offset);
    }
    let (hover, command) = ui.input(|input| (input.pointer.hover_pos(), input.modifiers.command));
    let pointer = hover.map(|pos| PointerState { pos, command });
    let mut navigate = None;
    scroll.show_rows(ui, row_height, source.lines.len(), |ui, range| {
        ui.set_min_width(total_width);
        for index in range {
            let (rect, response) = ui.allocate_exact_size(vec2(total_width, row_height), Sense::click());
            let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            let is_target = index + 1 == source.target_line;
            let style = LineStyle {
                font: &font,
                text_color: theme.text,
                background: if is_target { theme.selection } else { Color32::TRANSPARENT },
                gutter_background: theme.gutter_bg,
                line_number_color: if is_target { theme.accent } else { theme.text_faint },
                emphasis_background: Color32::TRANSPARENT,
                link_color: theme.accent,
            };
            let text = &source.lines[index];
            let line = CodeLine {
                number: Some(index + 1),
                text,
                visible: 0..text.len(),
                indent: 0.0,
                spans: source.spans.get(index).map(Vec::as_slice).unwrap_or(&[]),
                emphasis: &[],
            };
            let hit = paint_code_line(&painter, rect, gutter_width, &line, &style, pointer, 0.0);
            if hit.is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if response.clicked() {
                if let Some(hit) = hit {
                    navigate = Some(hit);
                }
            }
        }
    });
    navigate
}

const MAX_CANDIDATES_SHOWN: usize = 6;
const SCROLL_MARGIN: f32 = 60.0;
