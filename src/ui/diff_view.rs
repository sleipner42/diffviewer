use std::collections::HashSet;
use std::path::Path;

use egui::{Align2, Color32, CursorIcon, FontId, Rect, Response, RichText, Sense, TextStyle, Ui, pos2, vec2};

use super::code_line::{GUTTER_PADDING, LineStyle, PointerState, ROW_PADDING_Y, TEXT_PADDING, WordHit, paint_code_line};
use super::rendered::{RenderedBody, RenderedDiff, ViewRow};
use super::theme::Theme;
use crate::domain::diff::{DiffRow, LineSide, RowKind, SideBySideDiff};
use crate::infra::highlight::Span;

pub struct DiffViewState {
    pub collapsed: bool,
    pub expanded_gaps: HashSet<usize>,
    pub scroll_to_view_row: Option<usize>,
    pub current_hunk: Option<usize>,
}

impl DiffViewState {
    pub fn new(collapsed: bool) -> Self {
        Self {
            collapsed,
            expanded_gaps: HashSet::new(),
            scroll_to_view_row: None,
            current_hunk: None,
        }
    }

    pub fn reset_for_new_file(&mut self) {
        self.expanded_gaps.clear();
        self.scroll_to_view_row = None;
        self.current_hunk = None;
    }
}

pub fn show(
    ui: &mut Ui,
    rendered: &RenderedDiff,
    view_rows: &[ViewRow],
    state: &mut DiffViewState,
    theme: &Theme,
) -> Option<WordHit> {
    match &rendered.body {
        RenderedBody::Binary => {
            centered_message(ui, "Binary file", theme);
            None
        }
        RenderedBody::Text { diff, left, right, max_chars, max_line_number } => {
            if diff.rows.is_empty() {
                centered_message(ui, "Empty file", theme);
                return None;
            }
            let layout = compute_layout(ui, *max_chars, *max_line_number);
            show_rows(ui, &rendered.file.path, diff, left, right, view_rows, state, &layout, theme)
        }
    }
}

pub fn centered_message(ui: &mut Ui, text: &str, theme: &Theme) {
    ui.centered_and_justified(|ui| {
        ui.label(RichText::new(text).color(theme.text_dim).size(15.0));
    });
}

struct Layout {
    row_height: f32,
    gutter_width: f32,
    pane_width: f32,
    total_width: f32,
    font: FontId,
}

struct SideColors {
    background: Color32,
    emphasis: Color32,
    line_number: Color32,
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

fn compute_layout(ui: &Ui, max_chars: usize, max_line_number: usize) -> Layout {
    let font = TextStyle::Monospace.resolve(ui.style());
    let char_width = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, 'M'));
    let row_height = ui.text_style_height(&TextStyle::Monospace) + ROW_PADDING_Y * 2.0;
    let digits = max_line_number.max(1).to_string().len() as f32;
    let gutter_width = (digits + 1.0) * char_width + GUTTER_PADDING * 2.0;
    let text_needed = max_chars as f32 * char_width + TEXT_PADDING * 2.0;
    let available = ui.available_width();
    let pane_width = ((available - 2.0 * gutter_width - DIVIDER_WIDTH) / 2.0).max(text_needed);
    Layout {
        row_height,
        gutter_width,
        pane_width,
        total_width: 2.0 * (gutter_width + pane_width) + DIVIDER_WIDTH,
        font,
    }
}

#[allow(clippy::too_many_arguments)]
fn show_rows(
    ui: &mut Ui,
    path: &Path,
    diff: &SideBySideDiff,
    left_spans: &[Vec<Span>],
    right_spans: &[Vec<Span>],
    view_rows: &[ViewRow],
    state: &mut DiffViewState,
    layout: &Layout,
    theme: &Theme,
) -> Option<WordHit> {
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut scroll = egui::ScrollArea::both()
        .id_salt(("diff_body", path))
        .auto_shrink([false, false]);
    if let Some(row) = state.scroll_to_view_row.take() {
        let offset = (row as f32 * layout.row_height - SCROLL_MARGIN).max(0.0);
        scroll = scroll.vertical_scroll_offset(offset);
    }
    let (hover, command) = ui.input(|input| (input.pointer.hover_pos(), input.modifiers.command));
    let pointer = hover.map(|pos| PointerState { pos, command });
    let mut clicked = None;
    scroll.show_rows(ui, layout.row_height, view_rows.len(), |ui, range| {
        ui.set_min_width(layout.total_width);
        for index in range {
            match view_rows[index] {
                ViewRow::Line(row_index) => {
                    if let Some(row) = diff.rows.get(row_index) {
                        if let Some(hit) = paint_line_row(ui, row, left_spans, right_spans, layout, theme, pointer) {
                            clicked = Some(hit);
                        }
                    }
                }
                ViewRow::Gap { first, count } => {
                    if paint_gap_row(ui, count, layout, theme).clicked() {
                        state.expanded_gaps.insert(first);
                    }
                }
            }
        }
    });
    clicked
}

fn paint_line_row(
    ui: &mut Ui,
    row: &DiffRow,
    left_spans: &[Vec<Span>],
    right_spans: &[Vec<Span>],
    layout: &Layout,
    theme: &Theme,
    pointer: Option<PointerState>,
) -> Option<WordHit> {
    let (rect, response) = ui.allocate_exact_size(vec2(layout.total_width, layout.row_height), Sense::click());
    let side_size = vec2(layout.gutter_width + layout.pane_width, layout.row_height);
    let left_rect = Rect::from_min_size(rect.min, side_size);
    let right_rect = Rect::from_min_size(pos2(left_rect.right() + DIVIDER_WIDTH, rect.top()), side_size);
    let divider = Rect::from_min_max(pos2(left_rect.right(), rect.top()), pos2(right_rect.left(), rect.bottom()));
    ui.painter().rect_filled(divider, 0.0, theme.border);
    let left_hit = paint_side(ui, left_rect, row.left.as_ref(), side_colors(row.kind, Side::Left, theme), left_spans, layout, theme, pointer);
    let right_hit = paint_side(ui, right_rect, row.right.as_ref(), side_colors(row.kind, Side::Right, theme), right_spans, layout, theme, pointer);
    let hit = left_hit.or(right_hit);
    if hit.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if response.clicked() { hit } else { None }
}

fn side_colors(kind: RowKind, side: Side, theme: &Theme) -> SideColors {
    match (kind, side) {
        (RowKind::Unchanged, _) => SideColors {
            background: Color32::TRANSPARENT,
            emphasis: Color32::TRANSPARENT,
            line_number: theme.text_faint,
        },
        (RowKind::Added, _) | (RowKind::Modified, Side::Right) => SideColors {
            background: theme.added_bg,
            emphasis: theme.added_bg_strong,
            line_number: theme.added_fg,
        },
        (RowKind::Removed, _) | (RowKind::Modified, Side::Left) => SideColors {
            background: theme.removed_bg,
            emphasis: theme.removed_bg_strong,
            line_number: theme.removed_fg,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_side(
    ui: &Ui,
    rect: Rect,
    side: Option<&LineSide>,
    colors: SideColors,
    spans: &[Vec<Span>],
    layout: &Layout,
    theme: &Theme,
    pointer: Option<PointerState>,
) -> Option<WordHit> {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let Some(line) = side else {
        painter.rect_filled(rect, 0.0, theme.filler);
        return None;
    };
    let style = LineStyle {
        font: &layout.font,
        text_color: theme.text,
        background: colors.background,
        gutter_background: theme.gutter_bg,
        line_number_color: colors.line_number,
        emphasis_background: colors.emphasis,
        link_color: theme.accent,
    };
    let line_spans = spans.get(line.number - 1).map(Vec::as_slice).unwrap_or(&[]);
    paint_code_line(&painter, rect, layout.gutter_width, line.number, &line.text, line_spans, &line.emphasis, &style, pointer)
}

fn paint_gap_row(ui: &mut Ui, count: usize, layout: &Layout, theme: &Theme) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(layout.total_width, layout.row_height), Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() { theme.hover } else { theme.surface };
    painter.rect_filled(rect, 0.0, fill);
    let label_x = ui.clip_rect().left().max(rect.left()) + GAP_LABEL_OFFSET;
    painter.text(
        pos2(label_x, rect.center().y),
        Align2::LEFT_CENTER,
        format!("···  {count} unchanged lines  ·  click to expand"),
        FontId::proportional(11.0),
        theme.text_faint,
    );
    response
}

const DIVIDER_WIDTH: f32 = 2.0;
const SCROLL_MARGIN: f32 = 120.0;
const GAP_LABEL_OFFSET: f32 = 24.0;
