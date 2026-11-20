//! App-wide appearance preferences.

use super::visuals::theme::{Theme, set_theme};
use super::*;

impl VersusApp {
    pub(super) fn settings_dialog(&mut self, ctx: &egui::Context) {
        let current_theme = Theme::for_context(ctx);
        let mut theme = current_theme;
        let settings_button = egui::Id::new("Settings");
        let focus_selected = ctx.memory(|memory| memory.has_focus(settings_button));
        let available = ctx.content_rect();
        let dialog = egui::Modal::new(egui::Id::new("appearance-settings"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .inner_margin(12)
                    .corner_radius(6),
            )
            .show(ctx, |ui| {
                ui.set_width((available.width() - 40.0).clamp(100.0, 224.0));
                ui.label(RichText::new("Settings").size(14.0).strong());
                ui.add_space(8.0);
                ui.label(RichText::new("Theme").strong());
                egui::ScrollArea::vertical()
                    .max_height((available.height() - 100.0).max(30.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for option in Theme::ALL {
                            let response = ui.radio_value(&mut theme, option, option.label());
                            if focus_selected && option == current_theme {
                                response.request_focus();
                            }
                        }
                    });
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.button("Done").clicked()
                })
                .inner
            });
        if current_theme != theme {
            set_theme(ctx, theme);
        }
        if dialog.inner || dialog.should_close() {
            self.settings_open = false;
            ctx.memory_mut(|memory| memory.request_focus(settings_button));
        }
    }
}
