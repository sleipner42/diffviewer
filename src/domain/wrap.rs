use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapMode {
    Off,
    Columns(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLine {
    pub bytes: Range<usize>,
    pub indent_columns: usize,
}

pub fn wrap_line(text: &str, mode: WrapMode) -> Vec<VisualLine> {
    match mode {
        WrapMode::Columns(columns) if display_columns(text) > columns => break_into_lines(text, columns.max(1)),
        _ => vec![VisualLine { bytes: 0..text.len(), indent_columns: 0 }],
    }
}

pub fn visual_line_count(text: &str, mode: WrapMode) -> usize {
    match mode {
        WrapMode::Columns(columns) if display_columns(text) > columns => break_into_lines(text, columns.max(1)).len(),
        _ => 1,
    }
}

fn break_into_lines(text: &str, columns: usize) -> Vec<VisualLine> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let leading = chars.iter().take_while(|(_, character)| character.is_whitespace()).count();
    let leading_columns: usize = chars[..leading].iter().map(|(_, character)| char_columns(*character)).sum();
    let hanging_indent = (leading_columns + CONTINUATION_INDENT).min(columns / 2);
    let mut lines = Vec::new();
    let mut start = 0;
    let mut indent_columns = 0;
    let mut earliest_break = leading + 1;
    while start < chars.len() {
        let end = line_end(&chars, start, columns - indent_columns, earliest_break);
        let end_byte = chars.get(end).map_or(text.len(), |(offset, _)| *offset);
        lines.push(VisualLine { bytes: chars[start].0..end_byte, indent_columns });
        start = end;
        indent_columns = hanging_indent;
        earliest_break = start + 1;
    }
    lines
}

fn line_end(chars: &[(usize, char)], start: usize, capacity: usize, earliest_break: usize) -> usize {
    let limit = fitting_end(chars, start, capacity);
    let overflowing_spaces = chars[limit..].iter().take_while(|(_, character)| character.is_whitespace()).count();
    if overflowing_spaces > 0 || limit == chars.len() {
        return limit + overflowing_spaces;
    }
    (earliest_break..=limit)
        .rev()
        .find(|&index| can_break_before(chars, index))
        .unwrap_or(limit)
}

fn fitting_end(chars: &[(usize, char)], start: usize, capacity: usize) -> usize {
    let mut used = 0;
    let mut end = start;
    for (_, character) in &chars[start..] {
        used += char_columns(*character);
        if used > capacity && end > start {
            break;
        }
        end += 1;
    }
    end
}

fn can_break_before(chars: &[(usize, char)], index: usize) -> bool {
    let previous = chars[index - 1].1;
    let next = chars[index].1;
    !next.is_whitespace()
        && !NO_BREAK_BEFORE.contains(&next)
        && (previous.is_whitespace() || BREAK_AFTER.contains(&previous))
}

fn display_columns(text: &str) -> usize {
    text.chars().map(char_columns).sum()
}

fn char_columns(character: char) -> usize {
    if character == '\t' { TAB_COLUMNS } else { 1 }
}

const TAB_COLUMNS: usize = 4;
const CONTINUATION_INDENT: usize = 4;
const BREAK_AFTER: &[char] = &[',', ';', '(', '[', '{', '.', '/'];
const NO_BREAK_BEFORE: &[char] = &[')', ']', '}', ',', ';', ':', '.', '/'];

#[cfg(test)]
mod tests {
    use super::*;

    fn segments(text: &str, columns: usize) -> Vec<&str> {
        wrap_line(text, WrapMode::Columns(columns))
            .into_iter()
            .map(|line| &text[line.bytes])
            .collect()
    }

    fn indents(text: &str, columns: usize) -> Vec<usize> {
        wrap_line(text, WrapMode::Columns(columns))
            .into_iter()
            .map(|line| line.indent_columns)
            .collect()
    }

    #[test]
    fn keeps_a_fitting_line_whole() {
        assert_eq!(segments("short line", 10), vec!["short line"]);
        assert_eq!(indents("short line", 10), vec![0]);
    }

    #[test]
    fn keeps_an_empty_line_as_one_visual_line() {
        assert_eq!(segments("", 10), vec![""]);
        assert_eq!(visual_line_count("", WrapMode::Columns(10)), 1);
    }

    #[test]
    fn off_mode_never_breaks() {
        let text = "a".repeat(500);
        let lines = wrap_line(&text, WrapMode::Off);
        assert_eq!(lines, vec![VisualLine { bytes: 0..500, indent_columns: 0 }]);
        assert_eq!(visual_line_count(&text, WrapMode::Off), 1);
    }

    #[test]
    fn breaks_at_syntax_and_hangs_continuations_past_the_indent() {
        let text = "    return compute(alpha, beta, gamma)";
        assert_eq!(segments(text, 24), vec!["    return compute(", "alpha, beta, ", "gamma)"]);
        assert_eq!(indents(text, 24), vec![0, 8, 8]);
    }

    #[test]
    fn hard_breaks_a_token_without_break_points() {
        let text = "a".repeat(25);
        let lengths: Vec<usize> = segments(&text, 10).iter().map(|segment| segment.len()).collect();
        assert_eq!(lengths, vec![10, 6, 6, 3]);
    }

    #[test]
    fn does_not_break_inside_the_leading_indent() {
        let text = format!("        {}", "a".repeat(20));
        let first = segments(&text, 12)[0];
        assert_eq!(first, format!("        {}", "a".repeat(4)));
    }

    #[test]
    fn caps_the_hanging_indent_at_half_the_width() {
        let text = format!("        {}", "a".repeat(40));
        assert_eq!(indents(&text, 10)[1], 5);
    }

    #[test]
    fn counts_tabs_as_four_columns() {
        assert_eq!(segments("\tfoo bar", 8), vec!["\tfoo ", "bar"]);
        assert_eq!(indents("\tfoo bar", 8), vec![0, 4]);
        assert_eq!(segments("\tfoo", 8), vec!["\tfoo"]);
    }

    #[test]
    fn splits_on_char_boundaries() {
        assert_eq!(segments("åäö åäö åäö", 5), vec!["åäö ", "åäö ", "åäö"]);
        let text = "å".repeat(12);
        assert_eq!(segments(&text, 5).concat(), text);
    }

    #[test]
    fn overflowing_trailing_spaces_stay_on_their_line() {
        assert_eq!(segments("aaaa        ", 5), vec!["aaaa        "]);
        assert_eq!(segments("aaaaa    bbb", 5), vec!["aaaaa    ", "bbb"]);
    }

    #[test]
    fn keeps_closing_punctuation_with_its_opener() {
        assert_eq!(segments("aa bbbbb(), cc", 9)[0], "aa ");
    }

    #[test]
    fn narrowest_width_still_terminates() {
        assert_eq!(segments("abc", 0), vec!["a", "b", "c"]);
        assert_eq!(segments("\t\tx", 1).concat(), "\t\tx");
    }

    #[test]
    fn every_line_fits_and_nothing_is_lost() {
        let text = "        result = service.fetch_all(first_argument, second_argument)[index].value / divisor";
        for columns in 10..=text.len() {
            let lines = wrap_line(text, WrapMode::Columns(columns));
            let joined: String = lines.iter().map(|line| &text[line.bytes.clone()]).collect();
            assert_eq!(joined, text);
            assert_eq!(visual_line_count(text, WrapMode::Columns(columns)), lines.len());
            for line in &lines {
                let visible = text[line.bytes.clone()].trim_end();
                assert!(line.indent_columns + visible.chars().count() <= columns, "{visible:?} at {columns}");
            }
        }
    }
}
