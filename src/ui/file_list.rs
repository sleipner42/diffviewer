use std::path::{Path, PathBuf};

use egui::{Align2, Color32, CornerRadius, FontId, Painter, Rect, Response, Sense, Stroke, StrokeKind, TextStyle, Ui, pos2, vec2};

use super::theme::Theme;
use crate::domain::change::{ChangeKind, ChangedFile, StageState};
use crate::domain::importance::FileGroup;

pub enum FileListAction {
    Select(PathBuf),
    ToggleStage(PathBuf),
}

pub fn show(
    ui: &mut Ui,
    groups: &[&FileGroup],
    selected: Option<&Path>,
    scroll_to_selected: bool,
    theme: &Theme,
) -> Option<FileListAction> {
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("file_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if groups.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("Working tree clean").color(theme.text_dim));
                });
            }
            for group in groups {
                group_header(ui, group, theme);
                for file in &group.files {
                    let is_selected = selected == Some(file.path.as_path());
                    let response = file_row(ui, file, is_selected, theme);
                    if is_selected && scroll_to_selected {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() {
                        let on_checkbox = response.interact_pointer_pos().is_some_and(|pos| {
                            checkbox_rect(response.rect).expand(CHECKBOX_HIT_PADDING).contains(pos)
                        });
                        action = Some(if on_checkbox {
                            FileListAction::ToggleStage(file.path.clone())
                        } else {
                            FileListAction::Select(file.path.clone())
                        });
                    }
                }
                ui.add_space(GROUP_GAP);
            }
        });
    action
}

fn group_header(ui: &mut Ui, group: &FileGroup, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
    let staged = group.files.iter().filter(|file| file.is_fully_staged()).count();
    let title = format!("{}   {}/{}", group.title.to_uppercase(), staged, group.files.len());
    let color = if group.is_tests { theme.text_faint } else { theme.text_dim };
    ui.painter().text(
        pos2(rect.left() + ROW_PADDING, rect.center().y + 3.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(HEADER_FONT_SIZE),
        color,
    );
}

fn file_row(ui: &mut Ui, file: &ChangedFile, is_selected: bool, theme: &Theme) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let painter = ui.painter();
    if is_selected {
        painter.rect_filled(rect, 0.0, theme.selection);
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(2.0, rect.height())), 0.0, theme.accent);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, theme.hover);
    }
    paint_checkbox(painter, checkbox_rect(rect), file.stage, theme);

    let staged = file.is_fully_staged();
    let body = TextStyle::Body.resolve(ui.style());
    let small = TextStyle::Small.resolve(ui.style());
    let center_y = rect.center().y;

    let mut right_cursor = rect.right() - ROW_PADDING;
    if file.stats.removed > 0 {
        let drawn = painter.text(
            pos2(right_cursor, center_y),
            Align2::RIGHT_CENTER,
            format!("−{}", file.stats.removed),
            small.clone(),
            dim_if(staged, theme.removed_fg, theme),
        );
        right_cursor = drawn.left() - STAT_GAP;
    }
    if file.stats.added > 0 {
        let drawn = painter.text(
            pos2(right_cursor, center_y),
            Align2::RIGHT_CENTER,
            format!("+{}", file.stats.added),
            small.clone(),
            dim_if(staged, theme.added_fg, theme),
        );
        right_cursor = drawn.left() - STAT_GAP;
    }
    if let Some((label, color)) = kind_badge(&file.kind, theme) {
        let drawn = painter.text(
            pos2(right_cursor, center_y),
            Align2::RIGHT_CENTER,
            label,
            small.clone(),
            dim_if(staged, color, theme),
        );
        right_cursor = drawn.left() - STAT_GAP;
    }

    let name_color = if staged { theme.text_dim } else { theme.text };
    let name_rect = painter.text(
        pos2(rect.left() + NAME_OFFSET, center_y),
        Align2::LEFT_CENTER,
        file.file_name(),
        body,
        name_color,
    );
    let dir_x = name_rect.right() + DIR_GAP;
    let dir_max_width = right_cursor - DIR_GAP - dir_x;
    if dir_max_width > MIN_DIR_WIDTH {
        let dir = elide_start(painter, &file.directory(), &small, dir_max_width);
        painter.text(pos2(dir_x, center_y), Align2::LEFT_CENTER, dir, small, theme.text_faint);
    }
    response
}

fn kind_badge(kind: &ChangeKind, theme: &Theme) -> Option<(&'static str, Color32)> {
    match kind {
        ChangeKind::Modified => None,
        ChangeKind::Added | ChangeKind::Untracked => Some((kind.label(), theme.added_fg)),
        ChangeKind::Deleted => Some((kind.label(), theme.removed_fg)),
        ChangeKind::Renamed { .. } => Some((kind.label(), theme.modified_fg)),
    }
}

fn dim_if(dim: bool, color: Color32, theme: &Theme) -> Color32 {
    if dim { theme.text_faint } else { color }
}

fn paint_checkbox(painter: &Painter, rect: Rect, stage: StageState, theme: &Theme) {
    let corner = CornerRadius::same(3);
    match stage {
        StageState::Unstaged => {
            painter.rect_stroke(rect, corner, Stroke::new(1.0, theme.text_faint), StrokeKind::Inside);
        }
        StageState::Staged => {
            painter.rect_filled(rect, corner, theme.accent);
            let stroke = Stroke::new(1.8, theme.bg);
            let a = pos2(rect.left() + rect.width() * 0.25, rect.center().y);
            let b = pos2(rect.left() + rect.width() * 0.45, rect.bottom() - rect.height() * 0.28);
            let c = pos2(rect.right() - rect.width() * 0.22, rect.top() + rect.height() * 0.3);
            painter.line_segment([a, b], stroke);
            painter.line_segment([b, c], stroke);
        }
        StageState::Partial => {
            painter.rect_stroke(rect, corner, Stroke::new(1.0, theme.accent), StrokeKind::Inside);
            let y = rect.center().y;
            painter.line_segment(
                [pos2(rect.left() + 3.0, y), pos2(rect.right() - 3.0, y)],
                Stroke::new(1.8, theme.accent),
            );
        }
    }
}

fn checkbox_rect(row: Rect) -> Rect {
    Rect::from_center_size(
        pos2(row.left() + ROW_PADDING + CHECKBOX_SIZE / 2.0, row.center().y),
        vec2(CHECKBOX_SIZE, CHECKBOX_SIZE),
    )
}

fn elide_start(painter: &Painter, text: &str, font: &FontId, max_width: f32) -> String {
    let width = |candidate: &str| painter.layout_no_wrap(candidate.to_string(), font.clone(), Color32::WHITE).size().x;
    if width(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let with_ellipsis = |from: usize| -> String { std::iter::once(ELLIPSIS).chain(chars[from..].iter().copied()).collect() };
    let (mut low, mut high) = (0usize, chars.len());
    while low < high {
        let mid = (low + high) / 2;
        if width(&with_ellipsis(mid)) <= max_width {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    with_ellipsis(low.min(chars.len()))
}

const ROW_HEIGHT: f32 = 26.0;
const HEADER_HEIGHT: f32 = 26.0;
const HEADER_FONT_SIZE: f32 = 10.5;
const GROUP_GAP: f32 = 6.0;
const ROW_PADDING: f32 = 12.0;
const CHECKBOX_SIZE: f32 = 14.0;
const CHECKBOX_HIT_PADDING: f32 = 6.0;
const NAME_OFFSET: f32 = 36.0;
const DIR_GAP: f32 = 8.0;
const STAT_GAP: f32 = 6.0;
const MIN_DIR_WIDTH: f32 = 24.0;
const ELLIPSIS: char = '…';
