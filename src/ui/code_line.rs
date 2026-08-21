use std::ops::Range;

use egui::text::{CCursor, CharIndex, LayoutJob};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, TextFormat, pos2, vec2};

use crate::infra::highlight::Span;

pub struct LineStyle<'a> {
    pub font: &'a FontId,
    pub text_color: Color32,
    pub background: Color32,
    pub gutter_background: Color32,
    pub line_number_color: Color32,
    pub emphasis_background: Color32,
    pub link_color: Color32,
}

#[derive(Debug, Clone, Copy)]
pub struct PointerState {
    pub pos: Pos2,
    pub command: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordHit {
    pub word: String,
    pub attribute_access: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn paint_code_line(
    painter: &Painter,
    rect: Rect,
    gutter_width: f32,
    number: usize,
    text: &str,
    spans: &[Span],
    emphasis: &[Range<usize>],
    style: &LineStyle,
    pointer: Option<PointerState>,
) -> Option<WordHit> {
    let gutter = Rect::from_min_size(rect.min, vec2(gutter_width, rect.height()));
    if style.background != Color32::TRANSPARENT {
        painter.rect_filled(rect, 0.0, style.background);
    }
    painter.rect_filled(gutter, 0.0, style.gutter_background);
    painter.text(
        pos2(gutter.right() - GUTTER_PADDING, rect.center().y),
        Align2::RIGHT_CENTER,
        number.to_string(),
        style.font.clone(),
        style.line_number_color,
    );
    let job = line_job(text, spans, emphasis, style.font, style.text_color, style.emphasis_background);
    let galley = painter.layout_job(job);
    let text_pos = pos2(gutter.right() + TEXT_PADDING, rect.top() + ROW_PADDING_Y);
    let hit = pointer
        .filter(|pointer| pointer.command && rect.contains(pointer.pos) && pointer.pos.x >= text_pos.x)
        .and_then(|pointer| {
            let cursor = galley.cursor_from_pos(pointer.pos - text_pos);
            word_at_char(text, cursor.index.0)
        });
    if let Some((_, char_range)) = &hit {
        let left = galley.pos_from_cursor(CCursor::new(CharIndex(char_range.start))).left();
        let right = galley.pos_from_cursor(CCursor::new(CharIndex(char_range.end))).left();
        let y = text_pos.y + galley.size().y - 1.0;
        painter.line_segment(
            [pos2(text_pos.x + left, y), pos2(text_pos.x + right, y)],
            Stroke::new(1.0, style.link_color),
        );
    }
    painter.galley(text_pos, galley, style.text_color);
    hit.map(|(word_hit, _)| word_hit)
}

pub fn line_job(
    text: &str,
    spans: &[Span],
    emphasis: &[Range<usize>],
    font: &FontId,
    default_color: Color32,
    emphasis_background: Color32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    if text.is_empty() {
        return job;
    }
    let mut boundaries: Vec<usize> = vec![0, text.len()];
    boundaries.extend(spans.iter().flat_map(|span| [span.range.start, span.range.end]));
    boundaries.extend(emphasis.iter().flat_map(|range| [range.start, range.end]));
    boundaries.retain(|&offset| offset <= text.len() && text.is_char_boundary(offset));
    boundaries.sort_unstable();
    boundaries.dedup();
    for pair in boundaries.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let color = spans
            .iter()
            .find(|span| span.range.start <= start && start < span.range.end)
            .map(|span| Color32::from_rgb(span.color.r, span.color.g, span.color.b))
            .unwrap_or(default_color);
        let emphasized = emphasis.iter().any(|range| range.start <= start && start < range.end);
        job.append(
            &text[start..end],
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                background: if emphasized { emphasis_background } else { Color32::TRANSPARENT },
                ..Default::default()
            },
        );
    }
    job
}

fn word_at_char(text: &str, char_index: usize) -> Option<(WordHit, Range<usize>)> {
    let characters: Vec<char> = text.chars().collect();
    let anchor = char_index.min(characters.len().saturating_sub(1));
    if !characters.get(anchor).is_some_and(|&c| is_identifier_char(c)) {
        return None;
    }
    let mut start = anchor;
    while start > 0 && is_identifier_char(characters[start - 1]) {
        start -= 1;
    }
    let mut end = anchor + 1;
    while end < characters.len() && is_identifier_char(characters[end]) {
        end += 1;
    }
    let word: String = characters[start..end].iter().collect();
    if word.chars().next().is_some_and(|c| c.is_numeric()) {
        return None;
    }
    let attribute_access = start > 0 && characters[start - 1] == '.';
    Some((WordHit { word, attribute_access }, start..end))
}

fn is_identifier_char(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

pub const ROW_PADDING_Y: f32 = 2.0;
pub const GUTTER_PADDING: f32 = 8.0;
pub const TEXT_PADDING: f32 = 10.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_to_identifier_and_detects_attribute_access() {
        let text = "    traces = await service.get_agent_traces(sao_id, window)";
        let (hit, range) = word_at_char(text, text.find("agent").unwrap()).unwrap();
        assert_eq!(hit.word, "get_agent_traces");
        assert!(hit.attribute_access);
        assert_eq!(&text[range.start..range.end], "get_agent_traces");
        let (hit, _) = word_at_char(text, 4).unwrap();
        assert_eq!(hit.word, "traces");
        assert!(!hit.attribute_access);
    }

    #[test]
    fn ignores_punctuation_and_numbers() {
        assert!(word_at_char("x = foo(1)", 7).is_none());
        assert!(word_at_char("x = 42", 4).is_none());
        assert!(word_at_char("", 0).is_none());
    }
}
