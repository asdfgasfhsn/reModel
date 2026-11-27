//! The Cheats page (the pause menu's Cheats row, for a game whose poke table
//! has cheats: `romset::pokes`): one toggle row for each, with what it does
//! in small text, and a note that they are off each time a game starts.
//! Focus: `Menu::cheats_focus`.

use super::*;

/// What the page says about the cheats' lifetime.
pub(super) const NOTE: [(&str, bool); 3] = [
    ("Cheats are ", false),
    ("off each time a game starts", true),
    (". Saves made with one on are marked, and a cheat never changes the game's own settings or EEPROM.", false),
];

/// What the pause menu's Cheats row says in a network game.
pub(super) const NET_NOTE: &str = "Not in a network game: every machine must run the same game.";

/// A cheat's row: its label over its small line, a switch on the right.
/// Clicked, or activated from the pad.
fn cheat_row(ui: &mut egui::Ui, focus: &mut Focus, id: egui::Id, c: &CheatRow) -> bool {
    let s = theme::scale(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0 * s), Sense::hover());
    let r = ui.interact(rect, id, Sense::click());
    let p = ui.painter().clone();
    if focus.is_on(id) {
        theme::menu_row_focus(&p, rect, s);
    } else if r.hovered() {
        p.rect_filled(rect, theme::ROW_RADIUS * s, Color32::from_white_alpha(14));
    }
    let pad = 20.0 * s;
    let switch = Rect::from_center_size(pos2(rect.right() - pad - 21.0 * s, rect.center().y), vec2(42.0 * s, 24.0 * s));
    let width = (switch.left() - 14.0 * s - rect.left() - pad).max(10.0);
    let line = |text: &str, font: FontId, color: Color32| {
        let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let g1 = line(&c.label, FontId::new(18.0 * s, theme::semibold()), theme::TEXT);
    let g2 = line(&c.note, FontId::new(14.0 * s, FontFamily::Proportional), theme::TEXT_DIM);
    let top = rect.center().y - (g1.size().y + 2.0 * s + g2.size().y) / 2.0;
    let h1 = g1.size().y;
    p.galley(pos2(rect.left() + pad, top), g1, theme::TEXT);
    p.galley(pos2(rect.left() + pad, top + h1 + 2.0 * s), g2, theme::TEXT_DIM);
    let (fill, knob, x) = if c.on {
        (theme::YELLOW, Color32::from_rgb(17, 17, 17), switch.right() - 12.0 * s)
    } else {
        (Color32::from_white_alpha(30), theme::TEXT_2, switch.left() + 12.0 * s)
    };
    p.rect_filled(switch, 999.0, fill);
    p.circle_filled(pos2(x, switch.center().y), 8.5 * s, knob);
    focus.register(ui, &r)
}

impl Menu {
    pub(super) fn cheats_page(&mut self, ctx: &egui::Context, view: &View, navs: &[Nav], actions: &mut Vec<Action>) {
        // Not in a network game or link play, and nothing to show without a table.
        if view.cheats.is_empty() || view.running || view.link.is_some() {
            self.back_to_pause();
            return;
        }
        let s = theme::scale(ctx);
        let focus = &mut self.cheats_focus;
        focus.show = self.pad_used || self.keys_used;
        let mut back = focus.begin(navs).contains(&Nav::Back);
        let hints = [(Chip::A, "Select"), (Chip::B, "Back"), (Chip::Key("Esc"), "Resume")];
        let spec = PanelSpec { game: view.playing.unwrap_or("reModel"), heading: "Cheats", back: true, extra_h: 110.0 };
        back |= side_panel(ctx, &spec, &hints, |ui, reg| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(12.0 * s, 0.0))), |ui| {
                focus.scroll_area("cheats").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0 * s;
                    ui.add_space(8.0 * s);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0 * s);
                        ui.label(
                            egui::RichText::new("THIS GAME'S CHEATS")
                                .font(FontId::new(13.0 * s, theme::semibold()))
                                .color(theme::TEXT_DIM),
                        );
                    });
                    ui.add_space(4.0 * s);
                    for (i, c) in view.cheats.iter().enumerate() {
                        let id = ui.make_persistent_id(("cheat-row", i));
                        if cheat_row(ui, focus, id, c) {
                            actions.push(Action::Cheat(i));
                        }
                    }
                });
            });
            // The note, in a box at the foot.
            let note = reg.extra.shrink2(vec2(24.0 * s, 10.0 * s));
            ui.painter().rect_filled(note, theme::ROW_RADIUS * s, theme::RAISED);
            let mut job = egui::text::LayoutJob::default();
            for (text, strong) in NOTE {
                let (font, color) = if strong {
                    (FontId::new(14.0 * s, theme::bold()), theme::TEXT)
                } else {
                    (FontId::new(14.0 * s, FontFamily::Proportional), theme::TEXT_2)
                };
                job.append(text, 0.0, egui::TextFormat::simple(font, color));
            }
            job.wrap.max_width = note.width() - 28.0 * s;
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let at = pos2(note.left() + 14.0 * s, note.center().y - galley.size().y / 2.0);
            ui.painter().galley(at, galley, theme::TEXT_2);
            false
        });
        self.cheats_focus.end();
        if back {
            self.back_to_pause();
        }
    }
}
