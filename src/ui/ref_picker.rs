use egui::{Align, Button, Color32, Id, Key, Layout, Modal, RichText, ScrollArea, TextEdit, Ui};

use super::theme::Theme;
use crate::application::repository::{GitRef, RangeSpec, RefKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Base,
    Head,
}

pub enum PickerAction {
    Review(RangeSpec),
    Cancel,
}

pub struct RefPicker {
    refs: Vec<GitRef>,
    base: String,
    head: String,
    field: Field,
    highlighted: usize,
    /// Set for one frame after a field switch so the new field takes focus.
    focus_moved: bool,
}

impl RefPicker {
    pub fn new(refs: Vec<GitRef>, base: String, head: String) -> Self {
        Self {
            refs,
            base,
            head,
            field: Field::Base,
            highlighted: 0,
            focus_moved: true,
        }
    }

    fn query(&self) -> &str {
        match self.field {
            Field::Base => &self.base,
            Field::Head => &self.head,
        }
    }

    fn set_query(&mut self, value: String) {
        match self.field {
            Field::Base => self.base = value,
            Field::Head => self.head = value,
        }
    }

    /// Refs matching the active field, best first. A ref already typed in full
    /// still matches, so the list never goes empty under a complete name.
    fn matches(&self) -> Vec<&GitRef> {
        let query = self.query().trim().to_lowercase();
        let mut scored: Vec<(u8, &GitRef)> = self
            .refs
            .iter()
            .filter_map(|entry| rank(&entry.name.to_lowercase(), &query).map(|rank| (rank, entry)))
            .collect();
        scored.sort_by_key(|(rank, entry)| (*rank, entry.name.len()));
        scored.into_iter().map(|(_, entry)| entry).collect()
    }

    fn switch_field(&mut self, field: Field) {
        if self.field != field {
            self.field = field;
            self.highlighted = 0;
            self.focus_moved = true;
        }
    }

    fn accept_highlighted(&mut self) {
        let chosen = self.matches().get(self.highlighted).map(|entry| entry.name.clone());
        if let Some(name) = chosen {
            self.set_query(name);
        }
    }

    fn spec(&self) -> Option<RangeSpec> {
        let base = self.base.trim();
        if base.is_empty() {
            return None;
        }
        let head = self.head.trim();
        Some(RangeSpec {
            base: base.to_string(),
            head: if head.is_empty() { "HEAD".to_string() } else { head.to_string() },
        })
    }
}

pub fn show(ctx: &egui::Context, picker: &mut RefPicker, theme: &Theme) -> Option<PickerAction> {
    // Claim navigation keys before the text fields are drawn, so typing still
    // works but arrows and tab drive the list instead of the caret.
    // Only two fields, so shift-tab and tab both just toggle.
    let (up, down, tab, enter, escape) = ctx.input_mut(|input| {
        (
            input.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            input.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            input.consume_key(egui::Modifiers::NONE, Key::Tab),
            input.consume_key(egui::Modifiers::NONE, Key::Enter),
            input.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });

    let visible = picker.matches().len();
    if down && visible > 0 {
        picker.highlighted = (picker.highlighted + 1).min(visible - 1);
    }
    if up {
        picker.highlighted = picker.highlighted.saturating_sub(1);
    }
    if tab {
        picker.switch_field(match picker.field {
            Field::Base => Field::Head,
            Field::Head => Field::Base,
        });
    }

    let mut action = None;
    if escape {
        action = Some(PickerAction::Cancel);
    }
    if enter {
        // Take the highlighted ref, then advance; on the head field, start the
        // review. When nothing matches, the typed text is left alone, so any
        // revision git understands works even though it is not in the list --
        // `HEAD~3`, a raw sha, `main@{yesterday}`.
        picker.accept_highlighted();
        match picker.field {
            Field::Base => picker.switch_field(Field::Head),
            Field::Head => {
                if let Some(spec) = picker.spec() {
                    action = Some(PickerAction::Review(spec));
                }
            }
        }
    }

    let modal = Modal::new(Id::new("ref_picker")).show(ctx, |ui| {
        ui.set_width(MODAL_WIDTH);
        ui.heading("Review a range");
        ui.label(
            RichText::new("Compares the head against the point it forked from the base.")
                .small()
                .color(theme.text_dim),
        );
        ui.add_space(10.0);

        let mut inner = None;
        ui.horizontal(|ui| {
            field_edit(ui, picker, Field::Base, "Base", theme);
            ui.add_space(8.0);
            ui.label(RichText::new("…").color(theme.text_dim));
            ui.add_space(8.0);
            field_edit(ui, picker, Field::Head, "Head", theme);
        });
        ui.add_space(10.0);

        let entries = picker.matches();
        if entries.is_empty() {
            ui.label(RichText::new("No matching refs").color(theme.text_dim));
        }
        let highlighted = picker.highlighted.min(entries.len().saturating_sub(1));
        let mut clicked = None;
        ScrollArea::vertical()
            .id_salt("ref_list")
            .max_height(LIST_HEIGHT)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, entry) in entries.iter().enumerate() {
                    if ref_row(ui, entry, index == highlighted, theme) {
                        clicked = Some(entry.name.clone());
                    }
                }
            });
        if let Some(name) = clicked {
            picker.set_query(name);
            match picker.field {
                Field::Base => picker.switch_field(Field::Head),
                Field::Head => {
                    if let Some(spec) = picker.spec() {
                        inner = Some(PickerAction::Review(spec));
                    }
                }
            }
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("↑↓ select · ⇥ switch field · ↩ confirm · esc cancel")
                    .small()
                    .color(theme.text_faint),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ready = picker.spec().is_some();
                let review = Button::new(RichText::new("Review").color(if ready { theme.bg } else { theme.text_dim }))
                    .fill(if ready { theme.accent } else { theme.surface });
                if ui.add_enabled(ready, review).clicked() {
                    if let Some(spec) = picker.spec() {
                        inner = Some(PickerAction::Review(spec));
                    }
                }
                if ui.button("Cancel").clicked() {
                    inner = Some(PickerAction::Cancel);
                }
            });
        });
        inner
    });

    picker.highlighted = picker.highlighted.min(picker.matches().len().saturating_sub(1));
    picker.focus_moved = false;
    if modal.backdrop_response.clicked() {
        return Some(PickerAction::Cancel);
    }
    modal.inner.or(action)
}

fn field_edit(ui: &mut Ui, picker: &mut RefPicker, field: Field, label: &str, theme: &Theme) {
    let active = picker.field == field;
    ui.vertical(|ui| {
        let color = if active { theme.text } else { theme.text_dim };
        ui.label(RichText::new(label).small().color(color));
        let mut text = match field {
            Field::Base => picker.base.clone(),
            Field::Head => picker.head.clone(),
        };
        let edit = TextEdit::singleline(&mut text)
            .desired_width(FIELD_WIDTH)
            .hint_text(if field == Field::Head { "HEAD" } else { "base ref" });
        let response = ui.add(edit);
        if response.changed() {
            match field {
                Field::Base => picker.base = text,
                Field::Head => picker.head = text,
            }
            if picker.field == field {
                picker.highlighted = 0;
            }
        }
        if response.clicked() {
            picker.switch_field(field);
        }
        if active && picker.focus_moved {
            response.request_focus();
        }
    });
}

fn ref_row(ui: &mut Ui, entry: &GitRef, highlighted: bool, theme: &Theme) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::click());
    let painter = ui.painter();
    if highlighted {
        painter.rect_filled(rect, 0.0, theme.selection);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, theme.hover);
    }
    let center_y = rect.center().y;
    let small = egui::TextStyle::Small.resolve(ui.style());
    painter.text(
        egui::pos2(rect.left() + ROW_PADDING, center_y),
        egui::Align2::LEFT_CENTER,
        &entry.name,
        egui::TextStyle::Body.resolve(ui.style()),
        theme.text,
    );
    let mut right = rect.right() - ROW_PADDING;
    if !entry.age.is_empty() {
        let drawn = painter.text(
            egui::pos2(right, center_y),
            egui::Align2::RIGHT_CENTER,
            &entry.age,
            small.clone(),
            theme.text_faint,
        );
        right = drawn.left() - ROW_PADDING;
    }
    painter.text(
        egui::pos2(right, center_y),
        egui::Align2::RIGHT_CENTER,
        entry.kind.label(),
        small,
        kind_color(entry.kind, theme),
    );
    if highlighted {
        response.scroll_to_me(None);
    }
    response.clicked()
}

fn kind_color(kind: RefKind, theme: &Theme) -> Color32 {
    match kind {
        RefKind::Branch => theme.accent,
        RefKind::Remote => theme.text_dim,
        RefKind::Tag => theme.modified_fg,
    }
}

/// Lower is better. Exact match, then prefix, then substring, then subsequence,
/// so typing "mai" surfaces `main` above `origin/feature-maintenance`.
fn rank(name: &str, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(3);
    }
    if name == query {
        return Some(0);
    }
    if name.starts_with(query) {
        return Some(1);
    }
    if name.contains(query) {
        return Some(2);
    }
    if is_subsequence(name, query) {
        return Some(3);
    }
    None
}

fn is_subsequence(name: &str, query: &str) -> bool {
    let mut characters = name.chars();
    query.chars().all(|wanted| characters.any(|actual| actual == wanted))
}

const MODAL_WIDTH: f32 = 520.0;
const FIELD_WIDTH: f32 = 220.0;
const LIST_HEIGHT: f32 = 260.0;
const ROW_HEIGHT: f32 = 24.0;
const ROW_PADDING: f32 = 8.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn git_ref(name: &str, kind: RefKind) -> GitRef {
        GitRef { name: name.to_string(), kind, age: "1 day ago".to_string() }
    }

    fn picker() -> RefPicker {
        RefPicker::new(
            vec![
                git_ref("main", RefKind::Branch),
                git_ref("feat/range-review", RefKind::Branch),
                git_ref("origin/main", RefKind::Remote),
                git_ref("v0.2.1", RefKind::Tag),
            ],
            String::new(),
            String::new(),
        )
    }

    #[test]
    fn an_empty_query_offers_every_ref() {
        assert_eq!(picker().matches().len(), 4);
    }

    #[test]
    fn exact_and_prefix_matches_outrank_substring_ones() {
        let mut picker = picker();
        picker.base = "main".to_string();
        let names: Vec<&str> = picker.matches().iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names.first(), Some(&"main"), "exact match leads: {names:?}");
        assert!(names.contains(&"origin/main"), "substring still matches: {names:?}");
    }

    #[test]
    fn search_is_case_insensitive_and_subsequence_capable() {
        let mut picker = picker();
        picker.base = "RANGE".to_string();
        assert_eq!(picker.matches().len(), 1);
        picker.base = "frr".to_string();
        assert_eq!(picker.matches().first().map(|e| e.name.as_str()), Some("feat/range-review"));
    }

    #[test]
    fn head_defaults_to_head_when_left_blank() {
        let mut picker = picker();
        picker.base = "main".to_string();
        let spec = picker.spec().unwrap();
        assert_eq!(spec.base, "main");
        assert_eq!(spec.head, "HEAD");
    }

    #[test]
    fn a_revision_that_is_not_a_ref_is_left_intact() {
        // `HEAD~3` matches no ref, so accepting must not blank or replace it --
        // any revision git understands has to survive the picker.
        let mut picker = picker();
        picker.base = "HEAD~3".to_string();
        assert!(picker.matches().is_empty());
        picker.accept_highlighted();
        assert_eq!(picker.base, "HEAD~3");
        assert_eq!(picker.spec().unwrap().base, "HEAD~3");
    }

    #[test]
    fn a_blank_base_is_not_reviewable() {
        let mut picker = picker();
        picker.head = "feature".to_string();
        assert!(picker.spec().is_none());
    }

    #[test]
    fn the_active_field_is_the_one_being_searched() {
        let mut picker = picker();
        picker.base = "main".to_string();
        picker.switch_field(Field::Head);
        picker.head = "v0".to_string();
        assert_eq!(picker.matches().first().map(|e| e.name.as_str()), Some("v0.2.1"));
    }
}
