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

pub struct CodeLine<'a> {
    pub number: Option<usize>,
    pub text: &'a str,
    pub visible: Range<usize>,
    pub indent: f32,
    pub spans: &'a [Span],
    pub emphasis: &'a [Range<usize>],
}

pub fn paint_code_line(
    painter: &Painter,
    rect: Rect,
    gutter_width: f32,
    line: &CodeLine,
    style: &LineStyle,
    pointer: Option<PointerState>,
    scroll_x: f32,
) -> Option<WordHit> {
    let gutter = Rect::from_min_size(rect.min, vec2(gutter_width, rect.height()));
    if style.background != Color32::TRANSPARENT {
        painter.rect_filled(rect, 0.0, style.background);
    }
    painter.rect_filled(gutter, 0.0, style.gutter_background);
    if let Some(number) = line.number {
        painter.text(
            pos2(gutter.right() - GUTTER_PADDING, rect.center().y),
            Align2::RIGHT_CENTER,
            number.to_string(),
            style.font.clone(),
            style.line_number_color,
        );
    }
    let text_area = Rect::from_min_max(pos2(gutter.right(), rect.top()), rect.max);
    let text_painter = painter.with_clip_rect(text_area.intersect(painter.clip_rect()));
    let galley = text_painter.layout_job(line_job(line, style));
    let text_pos = pos2(gutter.right() + TEXT_PADDING + line.indent - scroll_x, rect.top() + ROW_PADDING_Y);
    let hit = pointer
        .filter(|pointer| pointer.command && text_area.contains(pointer.pos))
        .and_then(|pointer| {
            let cursor = galley.cursor_from_pos(pointer.pos - text_pos);
            word_in_visible_part(line.text, &line.visible, cursor.index.0)
        });
    if let Some((_, char_range)) = &hit {
        let left = galley.pos_from_cursor(CCursor::new(CharIndex(char_range.start))).left();
        let right = galley.pos_from_cursor(CCursor::new(CharIndex(char_range.end))).left();
        let y = text_pos.y + galley.size().y - 1.0;
        text_painter.line_segment(
            [pos2(text_pos.x + left, y), pos2(text_pos.x + right, y)],
            Stroke::new(1.0, style.link_color),
        );
    }
    text_painter.galley(text_pos, galley, style.text_color);
    hit.map(|(word_hit, _)| word_hit)
}

pub fn text_width(char_count: usize, char_width: f32) -> f32 {
    char_count as f32 * char_width + TEXT_PADDING * 2.0
}

fn line_job(line: &CodeLine, style: &LineStyle) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let visible = &line.visible;
    if visible.is_empty() {
        return job;
    }
    let mut boundaries: Vec<usize> = vec![visible.start, visible.end];
    boundaries.extend(line.spans.iter().flat_map(|span| [span.range.start, span.range.end]));
    boundaries.extend(line.emphasis.iter().flat_map(|range| [range.start, range.end]));
    boundaries.retain(|&offset| visible.start <= offset && offset <= visible.end && line.text.is_char_boundary(offset));
    boundaries.sort_unstable();
    boundaries.dedup();
    for pair in boundaries.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let color = line
            .spans
            .iter()
            .find(|span| span.range.start <= start && start < span.range.end)
            .map(|span| Color32::from_rgb(span.color.r, span.color.g, span.color.b))
            .unwrap_or(style.text_color);
        let emphasized = line.emphasis.iter().any(|range| range.start <= start && start < range.end);
        job.append(
            &line.text[start..end],
            0.0,
            TextFormat {
                font_id: style.font.clone(),
                color,
                background: if emphasized { style.emphasis_background } else { Color32::TRANSPARENT },
                ..Default::default()
            },
        );
    }
    job
}

fn word_in_visible_part(text: &str, visible: &Range<usize>, visible_char: usize) -> Option<(WordHit, Range<usize>)> {
    let visible_chars = text[visible.clone()].chars().count();
    let last = visible_chars.checked_sub(1)?;
    let offset = text[..visible.start].chars().count();
    let (hit, range) = word_at_char(text, offset + visible_char.min(last))?;
    Some((hit, range.start.saturating_sub(offset)..(range.end - offset).min(visible_chars)))
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
    fn resolves_a_word_from_a_wrapped_part_of_the_line() {
        let text = "first second_word third";
        let visible = text.find("second").unwrap()..text.find("third").unwrap();
        let (hit, range) = word_in_visible_part(text, &visible, 2).unwrap();
        assert_eq!(hit.word, "second_word");
        assert_eq!(range, 0.."second_word".len());
    }

    #[test]
    fn clamps_the_highlight_when_a_word_spans_two_visual_lines() {
        let text = "first second_word";
        let start = text.find("second").unwrap();
        let visible = start..start + "second".len();
        let (hit, range) = word_in_visible_part(text, &visible, 1).unwrap();
        assert_eq!(hit.word, "second_word");
        assert_eq!(range, 0.."second".len());
        let continuation = start + "second".len()..text.len();
        let (hit, range) = word_in_visible_part(text, &continuation, 1).unwrap();
        assert_eq!(hit.word, "second_word");
        assert_eq!(range, 0.."_word".len());
    }

    #[test]
    fn a_pointer_past_the_visual_line_stays_on_that_line() {
        let text = "first second";
        let visible = 0.."first ".len();
        assert!(word_in_visible_part(text, &visible, 99).is_none());
        let visible = 0.."first".len();
        assert_eq!(word_in_visible_part(text, &visible, 99).unwrap().0.word, "first");
    }

    #[test]
    fn a_blank_continuation_has_no_word() {
        assert!(word_in_visible_part("first second", &(0..0), 0).is_none());
    }

    #[test]
    fn ignores_punctuation_and_numbers() {
        assert!(word_at_char("x = foo(1)", 7).is_none());
        assert!(word_at_char("x = 42", 4).is_none());
        assert!(word_at_char("", 0).is_none());
    }
}
