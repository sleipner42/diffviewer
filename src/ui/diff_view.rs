use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

use egui::{Align2, Color32, CursorIcon, FontId, Rect, Response, RichText, Sense, TextStyle, Ui, pos2, vec2};

use super::code_line::{CodeLine, GUTTER_PADDING, LineStyle, PointerState, ROW_PADDING_Y, TEXT_PADDING, WordHit, paint_code_line, text_width};
use super::rendered::{RenderedBody, RenderedDiff, ViewRow};
use super::theme::Theme;
use super::visual_rows::VisualRows;
use crate::domain::diff::{DiffRow, LineSide, RowKind, SideBySideDiff};
use crate::domain::wrap::{VisualLine, WrapMode, wrap_line};
use crate::infra::highlight::Span;

pub struct DiffViewState {
    pub collapsed: bool,
    pub expanded_gaps: HashSet<usize>,
    pub scroll_to_view_row: Option<usize>,
    pub current_hunk: Option<usize>,
    pub scroll_x: f32,
    visual_rows: Option<VisualRows>,
    top_view_row: usize,
}

impl DiffViewState {
    pub fn new(collapsed: bool) -> Self {
        Self {
            collapsed,
            expanded_gaps: HashSet::new(),
            scroll_to_view_row: None,
            current_hunk: None,
            scroll_x: 0.0,
            visual_rows: None,
            top_view_row: 0,
        }
    }

    pub fn reset_for_new_file(&mut self) {
        self.expanded_gaps.clear();
        self.scroll_to_view_row = None;
        self.current_hunk = None;
        self.scroll_x = 0.0;
        self.visual_rows = None;
        self.top_view_row = 0;
    }

    pub fn invalidate_visual_rows(&mut self) {
        self.visual_rows = None;
    }
}

pub fn show(
    ui: &mut Ui,
    rendered: &RenderedDiff,
    view_rows: &[ViewRow],
    state: &mut DiffViewState,
    wrap_lines: bool,
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
            let layout = compute_layout(ui, *max_chars, *max_line_number, wrap_lines);
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
    char_width: f32,
    gutter_width: f32,
    pane_width: f32,
    total_width: f32,
    content_width: [f32; 2],
    wrap_mode: WrapMode,
    font: FontId,
}

impl Layout {
    fn widest_content(&self) -> f32 {
        self.content_width[0].max(self.content_width[1])
    }

    fn max_scroll(&self) -> f32 {
        (self.widest_content() - self.pane_width).max(0.0)
    }

    fn side_offset(&self, side: usize) -> f32 {
        side as f32 * (self.gutter_width + self.pane_width + DIVIDER_WIDTH)
    }
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

struct PaintContext<'a> {
    left_spans: &'a [Vec<Span>],
    right_spans: &'a [Vec<Span>],
    layout: &'a Layout,
    theme: &'a Theme,
    pointer: Option<PointerState>,
    scroll_x: f32,
}

struct SideLines<'a> {
    line: Option<&'a LineSide>,
    visual: Vec<VisualLine>,
    spans: &'a [Vec<Span>],
    colors: SideColors,
}

fn compute_layout(ui: &Ui, max_chars: [usize; 2], max_line_number: usize, wrap_lines: bool) -> Layout {
    let font = TextStyle::Monospace.resolve(ui.style());
    let char_width = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, 'M'));
    let row_height = ui.text_style_height(&TextStyle::Monospace) + ROW_PADDING_Y * 2.0;
    let digits = max_line_number.max(1).to_string().len() as f32;
    let gutter_width = (digits + 1.0) * char_width + GUTTER_PADDING * 2.0;
    let available = ui.available_width();
    let pane_width = ((available - 2.0 * gutter_width - DIVIDER_WIDTH) / 2.0)
        .max(MIN_PANE_WIDTH)
        .floor();
    let (wrap_mode, content_width) = if wrap_lines {
        let columns = ((pane_width - TEXT_PADDING * 2.0) / char_width).floor() as usize;
        (WrapMode::Columns(columns), [pane_width, pane_width])
    } else {
        (WrapMode::Off, [text_width(max_chars[0], char_width), text_width(max_chars[1], char_width)])
    };
    Layout {
        row_height,
        char_width,
        gutter_width,
        pane_width,
        total_width: 2.0 * (gutter_width + pane_width) + DIVIDER_WIDTH,
        content_width,
        wrap_mode,
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
    let viewport = ui.available_rect_before_wrap();
    route_horizontal_scroll(ui, viewport, layout, state);
    let (visual_rows, rewrapped) = current_visual_rows(state, diff, view_rows, layout.wrap_mode);
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("diff_body", path))
        .auto_shrink([false, false]);
    if let Some(offset) = pending_scroll_offset(state, &visual_rows, rewrapped, layout.row_height) {
        scroll = scroll.vertical_scroll_offset(offset);
    }
    let (hover, command) = ui.input(|input| (input.pointer.hover_pos(), input.modifiers.command));
    let context = PaintContext {
        left_spans,
        right_spans,
        layout,
        theme,
        pointer: hover.map(|pos| PointerState { pos, command }),
        scroll_x: state.scroll_x,
    };
    let mut clicked = None;
    let output = scroll.show_rows(ui, layout.row_height, visual_rows.total_lines(), |ui, range| {
        ui.set_min_width(layout.total_width);
        for visible in visual_rows.visible(range) {
            match view_rows[visible.view_row] {
                ViewRow::Line(row_index) => {
                    if let Some(row) = diff.rows.get(row_index) {
                        if let Some(hit) = paint_line_row(ui, row, visible.lines, &context) {
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
    state.top_view_row = visual_rows.view_row_at((output.state.offset.y / layout.row_height).round() as usize);
    state.visual_rows = Some(visual_rows);
    paint_horizontal_scrollbars(ui, output.inner_rect, layout, state, theme);
    clicked
}

fn current_visual_rows(
    state: &mut DiffViewState,
    diff: &SideBySideDiff,
    view_rows: &[ViewRow],
    wrap_mode: WrapMode,
) -> (VisualRows, bool) {
    match state.visual_rows.take() {
        Some(rows) if rows.mode() == wrap_mode => (rows, false),
        Some(_) => (VisualRows::build(diff, view_rows, wrap_mode), true),
        None => (VisualRows::build(diff, view_rows, wrap_mode), false),
    }
}

fn pending_scroll_offset(state: &mut DiffViewState, visual_rows: &VisualRows, rewrapped: bool, row_height: f32) -> Option<f32> {
    if let Some(row) = state.scroll_to_view_row.take() {
        return Some((visual_rows.first_line(row) as f32 * row_height - SCROLL_MARGIN).max(0.0));
    }
    rewrapped.then(|| visual_rows.first_line(state.top_view_row) as f32 * row_height)
}

fn route_horizontal_scroll(ui: &mut Ui, viewport: Rect, layout: &Layout, state: &mut DiffViewState) {
    state.scroll_x = state.scroll_x.clamp(0.0, layout.max_scroll());
    let (hover, delta) = ui.input(|input| (input.pointer.hover_pos(), input.smooth_scroll_delta));
    if delta.x == 0.0 || !hover.is_some_and(|pos| viewport.contains(pos)) {
        return;
    }
    let next = (state.scroll_x - delta.x).clamp(0.0, layout.max_scroll());
    if next != state.scroll_x {
        state.scroll_x = next;
        ui.input_mut(|input| input.smooth_scroll_delta.x = 0.0);
    }
}

fn paint_horizontal_scrollbars(ui: &mut Ui, inner_rect: Rect, layout: &Layout, state: &mut DiffViewState, theme: &Theme) {
    let max = layout.max_scroll();
    if max <= 0.0 {
        return;
    }
    for side in 0..2 {
        let track_left = inner_rect.left() + layout.side_offset(side) + layout.gutter_width;
        let track_right = (track_left + layout.pane_width).min(inner_rect.right());
        let track_width = track_right - track_left;
        if track_width <= MIN_THUMB_WIDTH {
            continue;
        }
        let y = inner_rect.bottom() - SCROLLBAR_MARGIN - SCROLLBAR_THICKNESS;
        let thumb_width = (track_width * layout.pane_width / layout.widest_content()).max(MIN_THUMB_WIDTH);
        let travel = track_width - thumb_width;
        let thumb_x = track_left + travel * (state.scroll_x / max);
        let thumb = Rect::from_min_size(pos2(thumb_x, y), vec2(thumb_width, SCROLLBAR_THICKNESS));
        let response = ui.interact(thumb.expand2(vec2(0.0, 4.0)), ui.id().with(("hscroll", side)), Sense::drag());
        if response.dragged() && travel > 0.0 {
            let delta = response.drag_delta().x / travel * max;
            state.scroll_x = (state.scroll_x + delta).clamp(0.0, max);
        }
        let track = Rect::from_min_size(pos2(track_left, y), vec2(track_width, SCROLLBAR_THICKNESS));
        let thumb_color = if response.hovered() || response.dragged() { theme.text_dim } else { theme.text_faint };
        let painter = ui.painter();
        painter.rect_filled(track, 2.0, theme.hover);
        painter.rect_filled(thumb, 2.0, thumb_color);
    }
}

fn paint_line_row(ui: &mut Ui, row: &DiffRow, lines: Range<usize>, context: &PaintContext) -> Option<WordHit> {
    let left = side_lines(row.left.as_ref(), context.left_spans, side_colors(row.kind, Side::Left, context.theme), context.layout.wrap_mode);
    let right = side_lines(row.right.as_ref(), context.right_spans, side_colors(row.kind, Side::Right, context.theme), context.layout.wrap_mode);
    let mut clicked = None;
    for index in lines {
        if let Some(hit) = paint_visual_line(ui, &left, &right, index, context) {
            clicked = Some(hit);
        }
    }
    clicked
}

fn side_lines<'a>(line: Option<&'a LineSide>, spans: &'a [Vec<Span>], colors: SideColors, wrap_mode: WrapMode) -> SideLines<'a> {
    let visual = line.map_or_else(Vec::new, |side| wrap_line(&side.text, wrap_mode));
    SideLines { line, visual, spans, colors }
}

fn paint_visual_line(ui: &mut Ui, left: &SideLines, right: &SideLines, index: usize, context: &PaintContext) -> Option<WordHit> {
    let layout = context.layout;
    let (rect, response) = ui.allocate_exact_size(vec2(layout.total_width, layout.row_height), Sense::click());
    let side_size = vec2(layout.gutter_width + layout.pane_width, layout.row_height);
    let left_rect = Rect::from_min_size(rect.min, side_size);
    let right_rect = Rect::from_min_size(pos2(rect.left() + layout.side_offset(1), rect.top()), side_size);
    let divider = Rect::from_min_max(pos2(left_rect.right(), rect.top()), pos2(right_rect.left(), rect.bottom()));
    ui.painter().rect_filled(divider, 0.0, context.theme.border);
    let left_hit = paint_side(ui, left_rect, left, index, context);
    let right_hit = paint_side(ui, right_rect, right, index, context);
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

fn paint_side(ui: &Ui, rect: Rect, side: &SideLines, index: usize, context: &PaintContext) -> Option<WordHit> {
    let (layout, theme) = (context.layout, context.theme);
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let Some(line) = side.line else {
        painter.rect_filled(rect, 0.0, theme.filler);
        return None;
    };
    let style = LineStyle {
        font: &layout.font,
        text_color: theme.text,
        background: side.colors.background,
        gutter_background: theme.gutter_bg,
        line_number_color: side.colors.line_number,
        emphasis_background: side.colors.emphasis,
        link_color: theme.accent,
    };
    let visual = side.visual.get(index);
    let code_line = CodeLine {
        number: (index == 0).then_some(line.number),
        text: &line.text,
        visible: visual.map_or(0..0, |visual| visual.bytes.clone()),
        indent: visual.map_or(0.0, |visual| visual.indent_columns as f32 * layout.char_width),
        spans: side.spans.get(line.number - 1).map(Vec::as_slice).unwrap_or(&[]),
        emphasis: &line.emphasis,
    };
    paint_code_line(&painter, rect, layout.gutter_width, &code_line, &style, context.pointer, context.scroll_x)
}

fn paint_gap_row(ui: &mut Ui, count: usize, layout: &Layout, theme: &Theme) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(layout.total_width, layout.row_height), Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() { theme.hover } else { theme.surface };
    painter.rect_filled(rect, 0.0, fill);
    painter.text(
        pos2(rect.left() + GAP_LABEL_OFFSET, rect.center().y),
        Align2::LEFT_CENTER,
        format!("···  {count} unchanged lines  ·  click to expand"),
        FontId::proportional(11.0),
        theme.text_faint,
    );
    response
}

const DIVIDER_WIDTH: f32 = 2.0;
const MIN_PANE_WIDTH: f32 = 200.0;
const SCROLL_MARGIN: f32 = 120.0;
const GAP_LABEL_OFFSET: f32 = 24.0;
const SCROLLBAR_THICKNESS: f32 = 4.0;
const SCROLLBAR_MARGIN: f32 = 4.0;
const MIN_THUMB_WIDTH: f32 = 24.0;

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput};

    use super::*;
    use crate::domain::diff::side_by_side;

    struct Harness {
        ctx: Context,
        diff: SideBySideDiff,
        view_rows: Vec<ViewRow>,
        state: DiffViewState,
        layout: Option<Layout>,
        time: f64,
    }

    impl Harness {
        fn new(old: &str, new: &str) -> Self {
            let diff = side_by_side(old, new);
            let view_rows = (0..diff.rows.len()).map(ViewRow::Line).collect();
            Self {
                ctx: Context::default(),
                diff,
                view_rows,
                state: DiffViewState::new(false),
                layout: None,
                time: 0.0,
            }
        }

        fn frame(&mut self, wrap_lines: bool, modifiers: Modifiers, events: Vec<Event>) -> Option<WordHit> {
            self.time += FRAME_SECONDS;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 600.0))),
                time: Some(self.time),
                predicted_dt: FRAME_SECONDS as f32,
                events: [vec![Event::ModifiersChanged(modifiers)], events].concat(),
                ..Default::default()
            };
            let theme = Theme::dark();
            let (left_chars, right_chars) = self.diff.max_line_chars_by_side();
            let max_line_number = self.diff.max_line_number();
            let mut hit = None;
            let Self { ctx, diff, view_rows, state, layout: last_layout, .. } = self;
            let mut output = ctx.run_ui(input, |ui| {
                let layout = compute_layout(ui, [left_chars, right_chars], max_line_number, wrap_lines);
                hit = show_rows(ui, Path::new("fixture.py"), diff, &[], &[], view_rows, state, &layout, &theme);
                *last_layout = Some(layout);
            });
            output.textures_delta.clear();
            hit
        }

        fn settle(&mut self, wrap_lines: bool) {
            self.frame(wrap_lines, Modifiers::NONE, Vec::new());
            self.frame(wrap_lines, Modifiers::NONE, Vec::new());
        }

        fn click(&mut self, wrap_lines: bool, modifiers: Modifiers, pos: Pos2) -> Option<WordHit> {
            let press = |pressed: bool| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
            self.frame(wrap_lines, modifiers, vec![Event::PointerMoved(pos)]);
            self.frame(wrap_lines, modifiers, vec![Event::PointerMoved(pos)]);
            self.frame(wrap_lines, modifiers, vec![press(true)]);
            self.frame(wrap_lines, modifiers, vec![press(false)])
        }

        fn layout(&self) -> &Layout {
            self.layout.as_ref().unwrap()
        }

        fn visual_rows(&self) -> &VisualRows {
            self.state.visual_rows.as_ref().unwrap()
        }
    }

    fn long_line(name: &str) -> String {
        let items: Vec<String> = (0..40).map(|index| format!("item_{index}")).collect();
        format!("{name} = call({})", items.join(", "))
    }

    fn long_file(changed_line: usize, changed_to: &str) -> String {
        let mut lines: Vec<String> = (0..200).map(|index| long_line(&format!("value_{index}"))).collect();
        lines[changed_line] = changed_to.to_string();
        lines.join("\n") + "\n"
    }

    #[test]
    fn hunk_jump_accounts_for_wrapped_rows_above_the_target() {
        let mut harness = Harness::new(&long_file(100, "before = 1"), &long_file(100, "after = 2"));
        harness.state.scroll_to_view_row = Some(100);
        harness.settle(true);
        let row_height = harness.layout().row_height;
        let first_line = harness.visual_rows().first_line(100);
        assert!(first_line > 100, "rows above the target should wrap");
        let expected_line = ((first_line as f32 * row_height - SCROLL_MARGIN) / row_height).round() as usize;
        assert_eq!(harness.state.top_view_row, harness.visual_rows().view_row_at(expected_line));
        assert!(harness.state.top_view_row < 100 && harness.state.top_view_row > 90);
    }

    #[test]
    fn toggling_wrap_keeps_the_top_row_anchored() {
        let mut harness = Harness::new(&long_file(100, "before = 1"), &long_file(100, "after = 2"));
        harness.state.scroll_to_view_row = Some(100);
        harness.settle(true);
        let anchored = harness.state.top_view_row;
        harness.settle(false);
        assert_eq!(harness.state.top_view_row, anchored);
        harness.settle(true);
        assert_eq!(harness.state.top_view_row, anchored);
    }

    #[test]
    fn wrapping_removes_horizontal_scroll_and_unwrapping_restores_it() {
        let mut harness = Harness::new(&long_file(0, "a"), &long_file(0, "b"));
        harness.state.scroll_x = 150.0;
        harness.settle(true);
        assert_eq!(harness.layout().max_scroll(), 0.0);
        assert_eq!(harness.state.scroll_x, 0.0);
        harness.state.scroll_x = 150.0;
        harness.settle(false);
        assert!(harness.layout().max_scroll() > 150.0);
        assert_eq!(harness.state.scroll_x, 150.0);
    }

    #[test]
    fn cmd_click_resolves_the_word_on_a_continuation_line() {
        let text = long_line("first");
        let mut harness = Harness::new(&format!("{text}\n"), &format!("{text}\n"));
        harness.settle(true);
        let layout = harness.layout();
        let continuation = wrap_line(&text, layout.wrap_mode).remove(1);
        let word: String = text[continuation.bytes.clone()].chars().take_while(|c| *c == '_' || c.is_alphanumeric()).collect();
        let x = layout.gutter_width + TEXT_PADDING + (continuation.indent_columns as f32 + 1.0) * layout.char_width;
        let pos = pos2(x, layout.row_height * 1.5);
        let hit = harness.click(true, Modifiers::COMMAND, pos).expect("a word under the pointer");
        assert_eq!(hit.word, word);
    }

    #[test]
    fn plain_click_on_a_continuation_line_does_nothing() {
        let text = long_line("first");
        let mut harness = Harness::new(&format!("{text}\n"), &format!("{text}\n"));
        harness.settle(true);
        let layout = harness.layout();
        let pos = pos2(layout.gutter_width + 40.0, layout.row_height * 1.5);
        assert_eq!(harness.click(true, Modifiers::NONE, pos), None);
    }

    #[test]
    fn the_shorter_side_pads_without_a_clickable_word() {
        let mut harness = Harness::new("short = 1\n", &format!("{}\n", long_line("short")));
        harness.settle(true);
        assert!(harness.visual_rows().total_lines() > 1);
        let layout = harness.layout();
        let pos = pos2(layout.gutter_width + TEXT_PADDING + 2.0 * layout.char_width, layout.row_height * 1.5);
        assert_eq!(harness.click(true, Modifiers::COMMAND, pos), None);
    }

    #[test]
    fn gap_rows_still_expand_when_clicked() {
        let mut harness = Harness::new("a\nb\nc\n", "a\nb\nc\n");
        harness.view_rows = vec![ViewRow::Gap { first: 0, count: 3 }];
        harness.settle(true);
        let pos = pos2(100.0, harness.layout().row_height * 0.5);
        harness.click(true, Modifiers::NONE, pos);
        assert!(harness.state.expanded_gaps.contains(&0));
    }

    #[test]
    fn rebuilt_rows_replace_the_cached_visual_rows_without_rescrolling() {
        let mut harness = Harness::new(&long_file(100, "before = 1"), &long_file(100, "after = 2"));
        harness.settle(true);
        harness.view_rows.truncate(1);
        harness.state.invalidate_visual_rows();
        harness.settle(true);
        assert!(harness.visual_rows().total_lines() < 50);
        assert_eq!(harness.state.top_view_row, 0);
    }

    #[test]
    fn a_huge_single_line_stays_responsive() {
        let huge: String = (0..8000).map(|index| format!("\"sku-{index}\", ")).collect();
        let mut harness = Harness::new(&format!("{huge}\n"), &format!("{huge}x\n"));
        let started = Instant::now();
        harness.settle(true);
        harness.settle(true);
        assert!(harness.visual_rows().total_lines() > 500);
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }

    const FRAME_SECONDS: f64 = 1.0 / 60.0;
}
