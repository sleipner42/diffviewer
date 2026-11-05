use std::ops::Range;

use super::rendered::ViewRow;
use crate::domain::diff::SideBySideDiff;
use crate::domain::wrap::{WrapMode, visual_line_count};

pub struct VisualRows {
    mode: WrapMode,
    starts: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleRow {
    pub view_row: usize,
    pub lines: Range<usize>,
}

impl VisualRows {
    pub fn build(diff: &SideBySideDiff, view_rows: &[ViewRow], mode: WrapMode) -> Self {
        let mut starts = Vec::with_capacity(view_rows.len() + 1);
        let mut total = 0;
        starts.push(total);
        for view_row in view_rows {
            total += line_count(diff, *view_row, mode);
            starts.push(total);
        }
        Self { mode, starts }
    }

    pub fn mode(&self) -> WrapMode {
        self.mode
    }

    pub fn total_lines(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }

    pub fn first_line(&self, view_row: usize) -> usize {
        self.starts.get(view_row).copied().unwrap_or_else(|| self.total_lines())
    }

    pub fn view_row_at(&self, line: usize) -> usize {
        let containing = self.starts.partition_point(|&start| start <= line).saturating_sub(1);
        containing.min(self.starts.len().saturating_sub(2))
    }

    pub fn visible(&self, lines: Range<usize>) -> Vec<VisibleRow> {
        let last_line = lines.end.min(self.total_lines());
        let mut rows = Vec::new();
        let mut line = lines.start;
        while line < last_line {
            let view_row = self.view_row_at(line);
            let first = self.first_line(view_row);
            let end = self.first_line(view_row + 1).min(last_line);
            rows.push(VisibleRow { view_row, lines: (line - first)..(end - first) });
            line = end;
        }
        rows
    }
}

fn line_count(diff: &SideBySideDiff, view_row: ViewRow, mode: WrapMode) -> usize {
    let ViewRow::Line(index) = view_row else {
        return 1;
    };
    let Some(row) = diff.rows.get(index) else {
        return 1;
    };
    [row.left.as_ref(), row.right.as_ref()]
        .into_iter()
        .flatten()
        .map(|side| visual_line_count(&side.text, mode))
        .max()
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::diff::side_by_side;

    fn fixture(mode: WrapMode) -> VisualRows {
        let diff = side_by_side("short\nlong line that wraps\nend\n", "short\nlong line that wraps twice over\nend\n");
        let view_rows: Vec<ViewRow> = (0..diff.rows.len()).map(ViewRow::Line).collect();
        VisualRows::build(&diff, &view_rows, mode)
    }

    #[test]
    fn unwrapped_rows_take_one_line_each() {
        let rows = fixture(WrapMode::Off);
        assert_eq!(rows.total_lines(), 3);
        assert_eq!(rows.first_line(2), 2);
        assert_eq!(rows.view_row_at(1), 1);
    }

    #[test]
    fn a_row_is_as_tall_as_its_taller_side() {
        let rows = fixture(WrapMode::Columns(10));
        assert_eq!(rows.first_line(1), 1);
        assert_eq!(rows.first_line(2), 6);
        assert_eq!(rows.total_lines(), 7);
    }

    #[test]
    fn maps_lines_back_to_their_row() {
        let rows = fixture(WrapMode::Columns(10));
        assert_eq!(rows.view_row_at(0), 0);
        assert_eq!(rows.view_row_at(1), 1);
        assert_eq!(rows.view_row_at(5), 1);
        assert_eq!(rows.view_row_at(6), 2);
        assert_eq!(rows.view_row_at(999), 2);
    }

    #[test]
    fn slices_a_range_that_starts_inside_a_wrapped_row() {
        let rows = fixture(WrapMode::Columns(10));
        assert_eq!(
            rows.visible(3..7),
            vec![
                VisibleRow { view_row: 1, lines: 2..5 },
                VisibleRow { view_row: 2, lines: 0..1 },
            ]
        );
    }

    #[test]
    fn range_past_the_end_is_clamped() {
        let rows = fixture(WrapMode::Off);
        assert_eq!(rows.visible(2..50), vec![VisibleRow { view_row: 2, lines: 0..1 }]);
        assert_eq!(rows.visible(50..60), Vec::new());
    }

    #[test]
    fn gap_rows_take_one_line() {
        let diff = side_by_side("a\nb\nc\n", "a\nb\nc\n");
        let rows = VisualRows::build(&diff, &[ViewRow::Gap { first: 0, count: 3 }], WrapMode::Columns(1));
        assert_eq!(rows.total_lines(), 1);
        assert_eq!(rows.view_row_at(0), 0);
    }
}
