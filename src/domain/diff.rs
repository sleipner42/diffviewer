use std::ops::Range;

use similar::{Algorithm, DiffOp, TextDiff};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Unchanged,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSide {
    pub number: usize,
    pub text: String,
    pub emphasis: Vec<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRow {
    pub kind: RowKind,
    pub left: Option<LineSide>,
    pub right: Option<LineSide>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SideBySideDiff {
    pub rows: Vec<DiffRow>,
    pub hunks: Vec<Hunk>,
}

impl SideBySideDiff {
    pub fn max_line_number(&self) -> usize {
        self.rows
            .iter()
            .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
            .flatten()
            .map(|side| side.number)
            .max()
            .unwrap_or(0)
    }

    pub fn max_line_chars(&self) -> usize {
        self.rows
            .iter()
            .flat_map(|row| [row.left.as_ref(), row.right.as_ref()])
            .flatten()
            .map(|side| side.text.chars().count())
            .max()
            .unwrap_or(0)
    }
}

pub fn side_by_side(old: &str, new: &str) -> SideBySideDiff {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .diff_lines(old, new);
    let old_lines: Vec<&str> = diff.iter_old_slices().collect();
    let new_lines: Vec<&str> = diff.iter_new_slices().collect();
    let mut rows = Vec::new();
    for op in diff.ops() {
        match *op {
            DiffOp::Equal { old_index, new_index, len } => {
                for offset in 0..len {
                    rows.push(DiffRow {
                        kind: RowKind::Unchanged,
                        left: Some(plain_side(old_index + offset, old_lines[old_index + offset])),
                        right: Some(plain_side(new_index + offset, new_lines[new_index + offset])),
                    });
                }
            }
            DiffOp::Delete { old_index, old_len, .. } => {
                push_removed(&mut rows, &old_lines, old_index, old_index + old_len);
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                push_added(&mut rows, &new_lines, new_index, new_index + new_len);
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                let paired = old_len.min(new_len);
                for offset in 0..paired {
                    rows.push(modified_row(
                        old_index + offset,
                        old_lines[old_index + offset],
                        new_index + offset,
                        new_lines[new_index + offset],
                    ));
                }
                push_removed(&mut rows, &old_lines, old_index + paired, old_index + old_len);
                push_added(&mut rows, &new_lines, new_index + paired, new_index + new_len);
            }
        }
    }
    let hunks = find_hunks(&rows);
    SideBySideDiff { rows, hunks }
}

fn push_removed(rows: &mut Vec<DiffRow>, lines: &[&str], from: usize, to: usize) {
    for index in from..to {
        rows.push(DiffRow {
            kind: RowKind::Removed,
            left: Some(plain_side(index, lines[index])),
            right: None,
        });
    }
}

fn push_added(rows: &mut Vec<DiffRow>, lines: &[&str], from: usize, to: usize) {
    for index in from..to {
        rows.push(DiffRow {
            kind: RowKind::Added,
            left: None,
            right: Some(plain_side(index, lines[index])),
        });
    }
}

fn modified_row(old_index: usize, old_raw: &str, new_index: usize, new_raw: &str) -> DiffRow {
    let old_text = strip_newline(old_raw);
    let new_text = strip_newline(new_raw);
    let (left_emphasis, right_emphasis) = inline_emphasis(old_text, new_text);
    DiffRow {
        kind: RowKind::Modified,
        left: Some(LineSide {
            number: old_index + 1,
            text: old_text.to_string(),
            emphasis: left_emphasis,
        }),
        right: Some(LineSide {
            number: new_index + 1,
            text: new_text.to_string(),
            emphasis: right_emphasis,
        }),
    }
}

fn plain_side(index: usize, raw: &str) -> LineSide {
    LineSide {
        number: index + 1,
        text: strip_newline(raw).to_string(),
        emphasis: Vec::new(),
    }
}

fn strip_newline(raw: &str) -> &str {
    raw.trim_end_matches(['\n', '\r'])
}

fn inline_emphasis(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let whole_lines = || (vec![0..old.len()], vec![0..new.len()]);
    let old_tokens = tokenize_code(old);
    let new_tokens = tokenize_code(new);
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .diff_slices(&old_tokens, &new_tokens);
    let old_offsets = token_offsets(old_tokens.iter().copied());
    let new_offsets = token_offsets(new_tokens.iter().copied());
    let offsets_consistent =
        old_offsets.last() == Some(&old.len()) && new_offsets.last() == Some(&new.len());
    if !offsets_consistent || diff.ratio() < MIN_INLINE_SIMILARITY {
        return whole_lines();
    }
    let mut left = Vec::new();
    let mut right = Vec::new();
    for op in diff.ops() {
        match *op {
            DiffOp::Equal { .. } => {}
            DiffOp::Delete { old_index, old_len, .. } => {
                left.push(old_offsets[old_index]..old_offsets[old_index + old_len]);
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                right.push(new_offsets[new_index]..new_offsets[new_index + new_len]);
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                left.push(old_offsets[old_index]..old_offsets[old_index + old_len]);
                right.push(new_offsets[new_index]..new_offsets[new_index + new_len]);
            }
        }
    }
    (merge_ranges(left), merge_ranges(right))
}

fn tokenize_code(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut current_class: Option<TokenClass> = None;
    for (offset, character) in text.char_indices() {
        let class = TokenClass::of(character);
        let continues_run = current_class == Some(class) && class != TokenClass::Punctuation;
        if !continues_run && offset > start {
            tokens.push(&text[start..offset]);
            start = offset;
        }
        current_class = Some(class);
    }
    if start < text.len() {
        tokens.push(&text[start..]);
    }
    tokens
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenClass {
    Word,
    Whitespace,
    Punctuation,
}

impl TokenClass {
    fn of(character: char) -> Self {
        if character.is_alphanumeric() || character == '_' {
            TokenClass::Word
        } else if character.is_whitespace() {
            TokenClass::Whitespace
        } else {
            TokenClass::Punctuation
        }
    }
}

fn token_offsets<'a>(tokens: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let mut offsets = vec![0];
    let mut total = 0;
    for token in tokens {
        total += token.len();
        offsets.push(total);
    }
    offsets
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if range.is_empty() {
            continue;
        }
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

fn find_hunks(rows: &[DiffRow]) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut open: Option<usize> = None;
    for (index, row) in rows.iter().enumerate() {
        let changed = row.kind != RowKind::Unchanged;
        match (changed, open) {
            (true, None) => open = Some(index),
            (false, Some(start)) => {
                hunks.push(Hunk { start, end: index });
                open = None;
            }
            _ => {}
        }
    }
    if let Some(start) = open {
        hunks.push(Hunk { start, end: rows.len() });
    }
    hunks
}

const MIN_INLINE_SIMILARITY: f32 = 0.35;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_replaced_lines_and_marks_inline_changes() {
        let diff = side_by_side("a\nfoo = bar(1)\nc\n", "a\nfoo = baz(1)\nc\n");
        assert_eq!(diff.rows.len(), 3);
        let row = &diff.rows[1];
        assert_eq!(row.kind, RowKind::Modified);
        let left = row.left.as_ref().unwrap();
        let right = row.right.as_ref().unwrap();
        assert_eq!(&left.text[left.emphasis[0].clone()], "bar");
        assert_eq!(&right.text[right.emphasis[0].clone()], "baz");
        assert_eq!(diff.hunks, vec![Hunk { start: 1, end: 2 }]);
    }

    #[test]
    fn pure_insert_leaves_left_empty() {
        let diff = side_by_side("a\n", "a\nb\n");
        assert_eq!(diff.rows[1].kind, RowKind::Added);
        assert!(diff.rows[1].left.is_none());
        assert_eq!(diff.rows[1].right.as_ref().unwrap().number, 2);
    }

    #[test]
    fn empty_inputs_produce_no_rows() {
        assert!(side_by_side("", "").rows.is_empty());
    }
}
