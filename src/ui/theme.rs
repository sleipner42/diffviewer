use egui::{Color32, Context, FontFamily, FontId, Stroke, TextStyle, ThemePreference, Visuals, vec2};

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub added_fg: Color32,
    pub removed_fg: Color32,
    pub modified_fg: Color32,
    pub added_bg: Color32,
    pub added_bg_strong: Color32,
    pub removed_bg: Color32,
    pub removed_bg_strong: Color32,
    pub filler: Color32,
    pub gutter_bg: Color32,
    pub selection: Color32,
    pub hover: Color32,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(30, 34, 39),
            panel: Color32::from_rgb(35, 39, 46),
            surface: Color32::from_rgb(42, 46, 54),
            border: Color32::from_rgb(22, 24, 28),
            text: Color32::from_rgb(171, 178, 191),
            text_dim: Color32::from_rgb(127, 132, 142),
            text_faint: Color32::from_rgb(92, 99, 112),
            accent: Color32::from_rgb(97, 175, 239),
            added_fg: Color32::from_rgb(152, 195, 121),
            removed_fg: Color32::from_rgb(224, 108, 117),
            modified_fg: Color32::from_rgb(229, 192, 123),
            added_bg: Color32::from_rgba_unmultiplied(152, 195, 121, 34),
            added_bg_strong: Color32::from_rgba_unmultiplied(152, 195, 121, 95),
            removed_bg: Color32::from_rgba_unmultiplied(224, 108, 117, 34),
            removed_bg_strong: Color32::from_rgba_unmultiplied(224, 108, 117, 95),
            filler: Color32::from_rgb(26, 29, 34),
            gutter_bg: Color32::from_rgba_unmultiplied(0, 0, 0, 40),
            selection: Color32::from_rgba_unmultiplied(97, 175, 239, 38),
            hover: Color32::from_rgba_unmultiplied(255, 255, 255, 10),
        }
    }

    pub fn apply(&self, ctx: &Context) {
        ctx.set_theme(ThemePreference::Dark);
        let mut visuals = Visuals::dark();
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.surface;
        visuals.extreme_bg_color = self.bg;
        visuals.faint_bg_color = self.surface;
        visuals.override_text_color = Some(self.text);
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text_dim);
        visuals.widgets.inactive.bg_fill = self.surface;
        visuals.widgets.inactive.weak_bg_fill = self.surface;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.text);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(52, 57, 66);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(52, 57, 66);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.text_faint);
        visuals.widgets.active.bg_fill = Color32::from_rgb(60, 66, 76);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(60, 66, 76);
        visuals.selection.bg_fill = self.selection;
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
        ctx.set_visuals(visuals);
        ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = vec2(8.0, 4.0);
            style.spacing.button_padding = vec2(10.0, 4.0);
            style.text_styles.insert(TextStyle::Body, FontId::new(BODY_SIZE, FontFamily::Proportional));
            style.text_styles.insert(TextStyle::Button, FontId::new(BODY_SIZE, FontFamily::Proportional));
            style.text_styles.insert(TextStyle::Small, FontId::new(SMALL_SIZE, FontFamily::Proportional));
            style.text_styles.insert(TextStyle::Heading, FontId::new(HEADING_SIZE, FontFamily::Proportional));
            style.text_styles.insert(TextStyle::Monospace, FontId::new(MONO_SIZE, FontFamily::Monospace));
        });
    }
}

const BODY_SIZE: f32 = 13.0;
const SMALL_SIZE: f32 = 11.0;
const HEADING_SIZE: f32 = 16.0;
const MONO_SIZE: f32 = 12.5;
