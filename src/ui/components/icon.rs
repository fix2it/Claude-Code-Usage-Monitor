use eframe::egui;
use lucide_icons::Icon as LucideIcon;

use crate::ui::tokens::CONTROL_HEIGHT;

pub(crate) fn icon_text(icon: LucideIcon, size: f32) -> egui::RichText {
    glyph_text(icon.unicode(), size)
}

/// Renders a bare codepoint from the `lucide` family, which also carries the
/// brand marks in [`crate::ui::theme`]. Prefer [`icon_text`] for Lucide icons.
pub(crate) fn glyph_text(glyph: char, size: f32) -> egui::RichText {
    egui::RichText::new(glyph.to_string())
        .family(egui::FontFamily::Name("lucide".into()))
        .size(size)
}

pub(crate) fn labeled_icon_button<'a>(icon: LucideIcon, label: &'a str) -> egui::Button<'a> {
    egui::Button::new((icon_text(icon, 16.0), label)).min_size(egui::vec2(0.0, CONTROL_HEIGHT))
}

pub(crate) fn icon_only_button(icon: LucideIcon) -> egui::Button<'static> {
    egui::Button::new(icon_text(icon, 16.0)).min_size(egui::vec2(CONTROL_HEIGHT, CONTROL_HEIGHT))
}

pub(crate) fn paint_centered_icon(
    ui: &egui::Ui,
    rect: egui::Rect,
    icon: LucideIcon,
    size: f32,
    color: egui::Color32,
) {
    ui.painter().text(
        rect.center() + egui::vec2(0.0, 0.5),
        egui::Align2::CENTER_CENTER,
        icon.unicode().to_string(),
        egui::FontId::new(size, egui::FontFamily::Name("lucide".into())),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::dropdown::Dropdown;
    use crate::ui::components::text_field::singleline;

    #[test]
    fn standard_controls_share_one_height() {
        let context = egui::Context::default();
        crate::ui::theme::configure_style(&context, crate::localization::LanguageId::English);
        let mut heights = [0.0; 6];
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| {
                heights[0] = Dropdown::from_id_salt("probe")
                    .selected_text("Theme")
                    .show_ui(ui, |_| {})
                    .response
                    .rect
                    .height();
                heights[1] = ui
                    .add(labeled_icon_button(LucideIcon::Save, "Apply"))
                    .rect
                    .height();
                heights[2] = ui.add(icon_only_button(LucideIcon::X)).rect.height();
                heights[3] = ui.button("Plain").rect.height();
                let mut text = String::new();
                heights[4] = ui.add(singleline(&mut text)).rect.height();
                heights[5] = ui
                    .add(egui::Button::new((
                        "Discard",
                        icon_text(LucideIcon::X, 16.0),
                    )))
                    .rect
                    .height();
            });
        });
        output.textures_delta.clear();
        assert_eq!(heights, [CONTROL_HEIGHT; 6]);
    }
}
