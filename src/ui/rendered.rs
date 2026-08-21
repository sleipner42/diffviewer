use std::collections::HashSet;

use crate::application::repository::FileStamp;
use crate::application::review_service::{DiffContent, FileDiff};
use crate::domain::change::ChangedFile;
use crate::domain::diff::SideBySideDiff;
use crate::infra::highlight::{Highlighter, Span};

pub struct RenderedDiff {
    pub file: ChangedFile,
    pub stamp: Option<FileStamp>,
    pub body: RenderedBody,
}

pub enum RenderedBody {
    Text {
        diff: SideBySideDiff,
        left: Vec<Vec<Span>>,
        right: Vec<Vec<Span>>,
        max_chars: usize,
        max_line_number: usize,
    },
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewRow {
    Line(usize),
    Gap { first: usize, count: usize },
}

impl RenderedDiff {
    pub fn build(file_diff: FileDiff, highlighter: &Highlighter) -> Self {
        let body = match file_diff.content {
            DiffContent::Binary => RenderedBody::Binary,
            DiffContent::Text(diff) => {
                let left = highlighter.highlight(file_diff.file.head_path(), &file_diff.old_text);
                let right = highlighter.highlight(&file_diff.file.path, &file_diff.new_text);
                RenderedBody::Text {
                    max_chars: diff.max_line_chars(),
                    max_line_number: diff.max_line_number(),
                    diff,
                    left,
                    right,
                }
            }
        };
        Self {
            file: file_diff.file,
            stamp: file_diff.stamp,
            body,
        }
    }

    pub fn diff(&self) -> Option<&SideBySideDiff> {
        match &self.body {
            RenderedBody::Text { diff, .. } => Some(diff),
            RenderedBody::Binary => None,
        }
    }
}

pub fn view_rows(diff: &SideBySideDiff, collapsed: bool, expanded: &HashSet<usize>) -> Vec<ViewRow> {
    if !collapsed {
        return (0..diff.rows.len()).map(ViewRow::Line).collect();
    }
    let mut visible = vec![false; diff.rows.len()];
    for hunk in &diff.hunks {
        let start = hunk.start.saturating_sub(CONTEXT_LINES);
        let end = (hunk.end + CONTEXT_LINES).min(diff.rows.len());
        visible[start..end].iter_mut().for_each(|flag| *flag = true);
    }
    let mut rows = Vec::new();
    let mut index = 0;
    while index < diff.rows.len() {
        if visible[index] {
            rows.push(ViewRow::Line(index));
            index += 1;
            continue;
        }
        let first = index;
        while index < diff.rows.len() && !visible[index] {
            index += 1;
        }
        let count = index - first;
        if count < MIN_GAP_ROWS || expanded.contains(&first) {
            rows.extend((first..index).map(ViewRow::Line));
        } else {
            rows.push(ViewRow::Gap { first, count });
        }
    }
    rows
}

const CONTEXT_LINES: usize = 3;
const MIN_GAP_ROWS: usize = 3;
