use std::path::PathBuf;

use egui::{Align2, Rect, Sense, TextStyle, Ui, pos2, vec2};

use super::keymap;
use super::theme::Theme;

pub struct TabInfo {
    pub name: String,
    pub root: PathBuf,
    pub changes: usize,
}

pub enum TabAction {
    Select(usize),
    Close(usize),
    Open,
}

pub fn show(ui: &mut Ui, tabs: &[TabInfo], active: usize, theme: &Theme) -> Option<TabAction> {
    let mut action = None;
    ui.spacing_mut().item_spacing.x = 0.0;
    ui.horizontal_centered(|ui| {
        ui.add_space(6.0);
        for (index, tab) in tabs.iter().enumerate() {
            if let Some(tab_action) = show_tab(ui, index, tab, index == active, theme) {
                action = Some(tab_action);
            }
        }
        ui.add_space(6.0);
        if ui.button("+").on_hover_text(format!("Open a folder in a new tab ({})", keymap::key_label("T"))).clicked() {
            action = Some(TabAction::Open);
        }
    });
    action
}

fn show_tab(ui: &mut Ui, index: usize, tab: &TabInfo, is_active: bool, theme: &Theme) -> Option<TabAction> {
    let body = TextStyle::Body.resolve(ui.style());
    let small = TextStyle::Small.resolve(ui.style());
    let number = format!("{}", index + 1);
    let changes = format!("{}", tab.changes);
    let measure = |text: &str, font: &egui::FontId| ui.painter().layout_no_wrap(text.to_string(), font.clone(), theme.text).size().x;
    let number_width = measure(&number, &small);
    let name_width = measure(&tab.name, &body);
    let changes_width = measure(&changes, &small);
    let width = TAB_PADDING + number_width + GAP + name_width + GAP + changes_width + GAP + CLOSE_SIZE + TAB_PADDING;
    let (rect, response) = ui.allocate_exact_size(vec2(width, ui.available_height()), Sense::click());
    let close_rect = Rect::from_center_size(
        pos2(rect.right() - TAB_PADDING - CLOSE_SIZE / 2.0, rect.center().y),
        vec2(CLOSE_SIZE, CLOSE_SIZE),
    );
    let close_response = ui.interact(close_rect, ui.id().with(("close_tab", index)), Sense::click());

    let painter = ui.painter();
    if is_active {
        painter.rect_filled(rect, 0.0, theme.bg);
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), ACTIVE_MARKER)), 0.0, theme.accent);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, theme.hover);
    }
    let text_color = if is_active { theme.text } else { theme.text_dim };
    let mut x = rect.left() + TAB_PADDING;
    let y = rect.center().y;
    painter.text(pos2(x, y), Align2::LEFT_CENTER, &number, small.clone(), theme.text_faint);
    x += number_width + GAP;
    painter.text(pos2(x, y), Align2::LEFT_CENTER, &tab.name, body, text_color);
    x += name_width + GAP;
    painter.text(pos2(x, y), Align2::LEFT_CENTER, &changes, small, theme.text_faint);
    let close_color = if close_response.hovered() { theme.text } else { theme.text_faint };
    painter.text(close_rect.center(), Align2::CENTER_CENTER, "×", TextStyle::Body.resolve(ui.style()), close_color);
    painter.line_segment(
        [pos2(rect.right(), rect.top() + 6.0), pos2(rect.right(), rect.bottom() - 6.0)],
        egui::Stroke::new(1.0, theme.border),
    );

    let selected = response.clicked();
    response.on_hover_text(tab.root.display().to_string());
    if close_response.clicked() {
        Some(TabAction::Close(index))
    } else if selected {
        Some(TabAction::Select(index))
    } else {
        None
    }
}

const TAB_PADDING: f32 = 12.0;
const GAP: f32 = 8.0;
const CLOSE_SIZE: f32 = 14.0;
const ACTIVE_MARKER: f32 = 2.0;
