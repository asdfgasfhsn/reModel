//! System settings' Players page: who plays, on which controller. For now
//! one placeholder row; the seats page replaces `Menu::players_page` (its
//! focus is `sys.players`).

use super::*;

impl Menu {
    pub(super) fn players_page(&mut self, ctx: &egui::Context, _view: &View, navs: &[Nav]) {
        let s = theme::scale(ctx);
        let focus = &mut self.sys.players;
        focus.show = self.pad_used || self.keys_used;
        let back = focus.begin(navs).contains(&Nav::Back);
        let hints = [(Chip::B, "Back")];
        let spec = PanelSpec { game: "System", heading: "Players", back: true, extra_h: 0.0 };
        let back = back
            | side_panel(ctx, &spec, &hints, |ui, reg| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(12.0 * s, 0.0))), |ui| {
                    focus.scroll_area("players").show(ui, |ui| {
                        ui.add_space(8.0 * s);
                        // (Nothing to do yet.)
                        focus.menu_row2(ui, "seats", "Seats", "P1 keyboard, pads join by pressing a button");
                    });
                });
                false
            });
        self.sys.players.end();
        if back {
            self.back_to_pause();
        }
    }
}
