use eframe::egui;

use crate::ui::tokens::CONTROL_HEIGHT;

/// A single-line text field that always occupies the shared control height.
pub(crate) struct SinglelineField<'a> {
    widget: egui::TextEdit<'a>,
    desired_width: f32,
}

pub(crate) fn singleline<'a>(text: &'a mut dyn egui::TextBuffer) -> SinglelineField<'a> {
    SinglelineField {
        widget: egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 6)),
        desired_width: f32::INFINITY,
    }
}

impl<'a> SinglelineField<'a> {
    pub(crate) fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = width;
        self.widget = self.widget.desired_width(width);
        self
    }

    pub(crate) fn hint_text(mut self, hint_text: impl Into<egui::WidgetText>) -> Self {
        self.widget = self.widget.hint_text(hint_text);
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui) -> egui::text_edit::TextEditOutput {
        let width = self.desired_width.min(ui.available_width()).max(1.0);
        let layout = egui::Layout::centered_and_justified(ui.layout().main_dir());
        ui.allocate_ui_with_layout(egui::vec2(width, CONTROL_HEIGHT), layout, |ui| {
            self.widget.show(ui)
        })
        .inner
    }
}

impl egui::Widget for SinglelineField<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        self.show(ui).response.response
    }
}
