use egui::{Button, RichText, TextStyle, Ui};

use super::theme::Theme;

pub enum CommitAction {
    Commit,
}

pub fn show(
    ui: &mut Ui,
    message: &mut String,
    staged_count: usize,
    request_focus: bool,
    theme: &Theme,
) -> Option<CommitAction> {
    let mut action = None;
    ui.add_space(6.0);
    let editor = egui::TextEdit::multiline(message)
        .desired_rows(MESSAGE_ROWS)
        .desired_width(f32::INFINITY)
        .hint_text("Commit message  (⌘↩ to commit)")
        .font(TextStyle::Body);
    let response = ui.add(editor);
    if request_focus {
        response.request_focus();
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let can_commit = staged_count > 0 && !message.trim().is_empty();
        let label = match staged_count {
            1 => "Commit 1 file".to_string(),
            count => format!("Commit {count} files"),
        };
        let text_color = if can_commit { theme.bg } else { theme.text_dim };
        let fill = if can_commit { theme.accent } else { theme.surface };
        let button = Button::new(RichText::new(label).color(text_color)).fill(fill);
        if ui.add_enabled(can_commit, button).clicked() {
            action = Some(CommitAction::Commit);
        }
        if staged_count == 0 {
            ui.label(RichText::new("nothing staged").color(theme.text_faint).small());
        }
    });
    ui.add_space(6.0);
    action
}

const MESSAGE_ROWS: usize = 3;
