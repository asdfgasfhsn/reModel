//! The Players screen (docs/play.md, "Players"): who plays which seat. A
//! card for each seat (P1 to P4 in the seat's colour, which side or which
//! machine it plays, and the device that holds it, with a lamp that lights
//! while that device is used) over a footer that says how many seats are
//! held and starts the game. A gamepad's first press takes the next free
//! seat (the player does that: `Seats::join_pad`); LB and RB (Q and E) move
//! the selected seat's player one seat over, Y (Delete) has them leave it.
//!
//! The screen is opened for the launcher's game page (a cabinet of two
//! players, or Play together on one screen), from the pause menu while a
//! game runs, and from the settings; it keeps no seats of its own (they are
//! the player's `seats::Seats`, kept between runs), only what it is showing.

use super::*;
use crate::seats::{self, Seats, Who};
use std::time::Duration;

/// What Start does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Goes {
    /// Start the game: one cabinet.
    Play(String),
    /// Start it as link play on this screen: this many machines.
    Link(String, usize),
    /// Just go back (the seats were the point): the pause menu's.
    Done,
}

pub enum Outcome {
    Go(Goes),
    Back,
}

/// What the screen needs from the player each frame.
pub struct PlayersView<'a> {
    /// The game's poster, if the launcher has it.
    pub poster: Option<egui::TextureId>,
    /// The keyboard's keys in a few words ("Arrows, X V Z").
    pub keys: &'a str,
}

/// The seat colours: P1 cyan, P2 red, P3 yellow, P4 green.
pub const SEAT_COLORS: [Color32; seats::MAX] = [theme::CYAN, theme::RED, theme::YELLOW, theme::GREEN];

pub struct PlayersScreen {
    pub game: &'static romset::Game,
    /// How many seats: a cabinet's players, or the machines on the screen.
    pub n: usize,
    /// Machines (link play) rather than the sides of one cabinet.
    pub link: bool,
    pub goes: Goes,
    /// The selected seat, or the Start button.
    sel: usize,
    on_button: bool,
    pad_used: bool,
    /// A line over the footer (who joined) and when it goes.
    note: Option<(String, Instant)>,
}

/// What a click or press asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    Select(usize),
    Leave,
    Join(usize),
    Go,
    Back,
    MoveBy(i32),
}

impl PlayersScreen {
    pub fn new(game: &'static romset::Game, n: usize, link: bool, goes: Goes, pad_used: bool) -> PlayersScreen {
        PlayersScreen { game, n: n.clamp(1, seats::MAX), link, goes, sel: 0, on_button: false, pad_used, note: None }
    }

    /// Say something under the seats for a moment.
    pub fn note(&mut self, text: impl Into<String>) {
        self.note = Some((text.into(), Instant::now()));
    }

    /// What the machines are called: cars in a racing game.
    fn noun(&self) -> &'static str {
        if self.game.kind == romset::Kind::Racing {
            "car"
        } else {
            "machine"
        }
    }

    /// The seat's place: the side of the cabinet, or the machine and where its picture is.
    fn tag(&self, i: usize) -> String {
        if !self.link {
            return if i == 0 { "Left side" } else { "Right side" }.to_string();
        }
        let noun = self.noun();
        let noun = format!("{}{}", noun[..1].to_uppercase(), &noun[1..]);
        let at = match (self.n, i) {
            (2, 0) => "left",
            (2, _) => "right",
            (_, 0) => "top left",
            (_, 1) => "top right",
            (_, 2) => "bottom left",
            _ => "bottom right",
        };
        format!("{noun} {} · {at}", i + 1)
    }

    fn context(&self) -> (&'static str, String) {
        if self.link {
            let noun = self.noun();
            ("Play together", format!("{} {noun}s on this screen. Each seat drives its own {noun}.", self.n))
        } else {
            ("One cabinet", format!("{} players. P1 plays the left side, P2 the right.", self.n))
        }
    }

    /// The footer's line, and the start button's label.
    fn status(&self, taken: usize) -> (String, String) {
        let n = self.n;
        let line = if self.link {
            if taken >= n {
                "Everyone is in.".to_string()
            } else {
                format!("{taken} of {n} seats held. Empty seats' {}s stay on the grid.", self.noun())
            }
        } else if taken >= n {
            "Ready.".to_string()
        } else {
            format!("{taken} of {n} seats held. A seat with no controller plays nothing.")
        };
        let label = match &self.goes {
            Goes::Play(_) => "Play".to_string(),
            Goes::Link(..) => format!("Start, {taken} player{}", if taken == 1 { "" } else { "s" }),
            Goes::Done => "Done".to_string(),
        };
        (line, label)
    }

    fn apply<I: Copy + PartialEq>(&mut self, cmd: Cmd, seats: &mut Seats<I>) -> Option<Outcome> {
        match cmd {
            Cmd::Select(i) => {
                self.sel = i.min(self.n - 1);
                self.on_button = false;
            }
            Cmd::Leave => {
                if self.on_button {
                    return None;
                }
                if let Some((name, _)) = seats.describe(self.sel) {
                    seats.leave(self.sel);
                    self.note(format!("{name} left seat P{}", self.sel + 1));
                }
            }
            Cmd::Join(i) => {
                self.sel = i;
                if seats.join_keyboard_at(i, self.n) {
                    self.note(format!("Keyboard took seat P{}", i + 1));
                }
            }
            Cmd::MoveBy(d) => {
                let to = self.sel as i32 + d;
                if !self.on_button && to >= 0 && seats.move_seat(self.sel, to as usize, self.n) {
                    let name = seats.describe(to as usize).map_or(String::new(), |d| d.0);
                    self.sel = to as usize;
                    self.note(format!("{name} moved to P{}", to + 1));
                }
            }
            Cmd::Go => return Some(Outcome::Go(self.goes.clone())),
            Cmd::Back => return Some(Outcome::Back),
        }
        None
    }

    /// Draw the screen over everything and act on the input: `navs` are the pad's, the
    /// keyboard's are read here. The outcome, once it has one.
    pub fn ui<I: Copy + PartialEq>(
        &mut self,
        ctx: &egui::Context,
        navs: &[Nav],
        seats: &mut Seats<I>,
        view: &PlayersView,
    ) -> Option<Outcome> {
        let keys = Launcher::keys(ctx);
        let (join, delete) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::J),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
            )
        });
        if !navs.is_empty() {
            self.pad_used = true;
        } else if !keys.is_empty() || join || delete {
            self.pad_used = false;
        }
        let mut cmds: Vec<Cmd> = Vec::new();
        // (The keyboard's C is the launcher's Continue: not leaving a seat; Delete is.)
        for &nav in Nav::view_as_back(navs).iter().chain(keys.iter().filter(|&&k| k != Nav::Continue)) {
            let n = self.n;
            match nav {
                Nav::Left if !self.on_button => self.sel = self.sel.saturating_sub(1),
                Nav::Right if !self.on_button => self.sel = (self.sel + 1).min(n - 1),
                Nav::Down => self.on_button = true,
                Nav::Up => self.on_button = false,
                Nav::PrevGroup => cmds.push(Cmd::MoveBy(-1)),
                Nav::NextGroup => cmds.push(Cmd::MoveBy(1)),
                Nav::Continue => cmds.push(Cmd::Leave),
                Nav::Accept => cmds.push(Cmd::Go),
                Nav::Back => cmds.push(Cmd::Back),
                _ => {}
            }
        }
        if delete {
            cmds.push(Cmd::Leave);
        }
        if join && seats.keyboard_seat().is_none() {
            let at = if seats.held(self.sel).is_none() {
                Some(self.sel)
            } else {
                (0..self.n).find(|&i| seats.held(i).is_none())
            };
            cmds.extend(at.map(Cmd::Join));
        }
        // The lamps: a keyboard key lights the keyboard's.
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
            seats.used(Who::Keyboard);
        }
        if self.note.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(3500)) {
            self.note = None;
        }

        let l = layout(ctx.content_rect());
        let u = l.u;
        let mut clicks: Vec<Cmd> = Vec::new();
        let taken = seats.taken(self.n);
        let (status, label) = self.status(taken);
        let any_lit = (0..self.n).any(|i| seats.lit(i));
        egui::Area::new(Id::unique("players screen"))
            .fixed_pos(l.screen.min)
            .order(egui::Order::Tooltip)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_min_size(l.screen.size());
                let p = ui.painter().clone();
                // The page: dark, over whatever is behind.
                p.rect_filled(l.screen, 0.0, theme::BG);
                ui.allocate_rect(l.screen, Sense::click_and_drag());
                self.paint_header(&p, &l, view);
                let (mode, text) = self.context();
                let y = l.screen.top() + 150.0 * u;
                let g = p.layout_no_wrap(mode.to_uppercase(), FontId::new(14.0 * u, theme::bold()), theme::CYAN);
                let pill = Rect::from_min_size(pos2(l.top.left(), y), g.size() + vec2(20.0 * u, 12.0 * u));
                p.rect_filled(pill, 6.0 * u, Color32::from_rgba_unmultiplied(34, 229, 255, 38));
                p.galley(pill.min + vec2(10.0 * u, 6.0 * u), g, theme::CYAN);
                let font = FontId::new(20.0 * u, FontFamily::Proportional);
                let text = fit_text(&p, &text, &font, l.top.width() - pill.width() - 20.0 * u);
                p.text(pos2(pill.right() + 16.0 * u, pill.center().y), Align2::LEFT_CENTER, text, font, theme::TEXT_2);

                // The seat cards.
                let foot_h = 72.0 * u;
                let cards_top = y + 56.0 * u;
                let cards_bottom = (l.bar.top() - foot_h - 40.0 * u).max(cards_top + 160.0 * u);
                let gap = 20.0 * u;
                let w = (l.top.width() - gap * (self.n as f32 - 1.0)) / self.n as f32;
                let h = (cards_bottom - cards_top).min(280.0 * u);
                for i in 0..self.n {
                    let rect = Rect::from_min_size(pos2(l.top.left() + i as f32 * (w + gap), cards_top), vec2(w, h));
                    self.paint_seat(ui, &p, rect, i, seats, view, u, &mut clicks);
                }

                // The footer.
                let foot = Rect::from_min_size(
                    pos2(l.top.left(), l.bar.top() - foot_h - 16.0 * u),
                    vec2(l.top.width(), foot_h),
                );
                p.rect_filled(foot, 14.0 * u, Color32::from_white_alpha(20));
                let button = Rect::from_center_size(
                    pos2(foot.right() - 20.0 * u - 130.0 * u, foot.center().y),
                    vec2(260.0 * u, 48.0 * u),
                );
                let font = FontId::new(19.0 * u, FontFamily::Proportional);
                let line = match &self.note {
                    Some((n, _)) => n.clone(),
                    None => status,
                };
                let line = fit_text(&p, &line, &font, button.left() - foot.left() - 52.0 * u);
                p.text(pos2(foot.left() + 22.0 * u, foot.center().y), Align2::LEFT_CENTER, line, font, theme::TEXT_2);
                let r = ui.interact(button, Id::unique("players start"), Sense::click());
                p.rect_filled(
                    button,
                    12.0 * u,
                    if r.hovered() { Color32::from_rgb(60, 130, 255) } else { theme::BLUE },
                );
                p.rect_stroke(
                    button,
                    12.0 * u,
                    Stroke::new(2.0 * u, Color32::from_rgba_unmultiplied(34, 229, 255, 128)),
                    StrokeKind::Inside,
                );
                if self.on_button {
                    theme::focus_ring(&p, button, u, 12.0);
                }
                let font = FontId::new(21.0 * u, theme::bold());
                p.text(
                    button.center(),
                    Align2::CENTER_CENTER,
                    fit_text(&p, &label, &font, button.width() - 20.0 * u),
                    font,
                    FG,
                );
                if r.clicked() {
                    clicks.push(Cmd::Go);
                }
                self.paint_bar(ui, &p, &l, seats, &label, &mut clicks);
            });
        if any_lit {
            ctx.request_repaint_after(seats::LAMP);
        }
        if self.note.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        for cmd in cmds.into_iter().chain(clicks) {
            if let Some(out) = self.apply(cmd, seats) {
                return Some(out);
            }
        }
        None
    }

    fn paint_header(&self, p: &Painter, l: &Layout, view: &PlayersView) {
        let u = l.u;
        let top = l.screen.top() + 36.0 * u;
        let mut x = l.top.left();
        if let Some(tex) = view.poster {
            let r = Rect::from_min_size(pos2(x, top), vec2(64.0 * u, 90.0 * u));
            p.add(egui::epaint::RectShape::filled(r, 8.0 * u, Color32::WHITE).with_texture(tex, full_uv()));
            x = r.right() + 18.0 * u;
        }
        p.text(
            pos2(x, top + 4.0 * u),
            Align2::LEFT_TOP,
            self.game.title.to_uppercase(),
            FontId::new(16.0 * u, theme::semibold()),
            theme::CYAN_LIGHT,
        );
        p.text(pos2(x, top + 28.0 * u), Align2::LEFT_TOP, "PLAYERS", FontId::new(42.0 * u, theme::display()), FG);
    }

    /// One seat's card.
    #[allow(clippy::too_many_arguments)]
    fn paint_seat<I: Copy + PartialEq>(
        &self,
        ui: &mut Ui,
        p: &Painter,
        rect: Rect,
        i: usize,
        seats: &Seats<I>,
        view: &PlayersView,
        u: f32,
        clicks: &mut Vec<Cmd>,
    ) {
        let color = SEAT_COLORS[i];
        let r = ui.interact(rect, Id::unique(("players seat", i)), Sense::click());
        if r.clicked() {
            clicks.push(if seats.held(i).is_none() && seats.keyboard_seat().is_none() {
                Cmd::Join(i)
            } else {
                Cmd::Select(i)
            });
        }
        let held = seats.describe(i);
        let sel = self.sel == i && !self.on_button;
        let radius = 18.0 * u;
        if held.is_some() {
            p.rect_filled(rect, radius, Color32::from_white_alpha(if r.hovered() { 30 } else { 22 }));
        } else {
            p.rect_filled(rect, radius, Color32::from_white_alpha(8));
        }
        if sel {
            p.rect_stroke(rect, radius, Stroke::new(2.5 * u, theme::CYAN), StrokeKind::Inside);
        } else if held.is_none() {
            let pts = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
            p.extend(egui::Shape::dashed_line(
                &pts,
                Stroke::new(1.6 * u, Color32::from_white_alpha(46)),
                10.0 * u,
                7.0 * u,
            ));
        }
        let pad = 22.0 * u;
        let ink = if held.is_some() { color } else { color.gamma_multiply(0.35) };
        p.text(
            pos2(rect.left() + pad, rect.top() + 14.0 * u),
            Align2::LEFT_TOP,
            format!("P{}", i + 1),
            FontId::new(66.0 * u, theme::display()),
            ink,
        );
        let tag_font = FontId::new(12.0 * u, theme::bold());
        let tag = fit_text(p, &self.tag(i).to_uppercase(), &tag_font, rect.width() - pad * 2.0 - 80.0 * u);
        p.text(pos2(rect.right() - pad, rect.top() + 30.0 * u), Align2::RIGHT_TOP, tag, tag_font, DIM);
        let strip = Rect::from_min_size(
            pos2(rect.left() + pad * 0.8, rect.bottom() - 92.0 * u),
            vec2(rect.width() - pad * 1.6, 72.0 * u),
        );
        match held {
            Some((name, detail)) => {
                p.rect_filled(strip, 12.0 * u, Color32::from_white_alpha(14));
                let icon = Rect::from_min_size(strip.min + vec2(12.0 * u, 14.0 * u), vec2(60.0 * u, 44.0 * u));
                p.rect_filled(icon, 8.0 * u, color);
                let kb = seats.held(i) == Some(Who::Keyboard);
                p.text(
                    icon.center(),
                    Align2::CENTER_CENTER,
                    if kb { "KEYS" } else { "PAD" },
                    FontId::new(15.0 * u, theme::bold()),
                    theme::INK,
                );
                let lamp = pos2(strip.right() - 18.0 * u, strip.center().y);
                let lit = seats.lit(i);
                if lit {
                    p.circle_filled(lamp, 12.0 * u, theme::GREEN.gamma_multiply(0.25));
                }
                p.circle_filled(lamp, 6.5 * u, if lit { theme::GREEN } else { Color32::from_white_alpha(40) });
                let room = lamp.x - 16.0 * u - (icon.right() + 14.0 * u);
                let detail = if kb { view.keys.to_string() } else { detail };
                let font = FontId::new(20.0 * u, theme::bold());
                let name = fit_text(p, &name, &font, room);
                let small = FontId::new(14.0 * u, FontFamily::Proportional);
                let two = !detail.is_empty();
                let x = icon.right() + 14.0 * u;
                p.text(
                    pos2(x, strip.center().y - if two { 11.0 * u } else { 0.0 }),
                    Align2::LEFT_CENTER,
                    name,
                    font,
                    FG,
                );
                if two {
                    p.text(
                        pos2(x, strip.center().y + 13.0 * u),
                        Align2::LEFT_CENTER,
                        fit_text(p, &detail, &small, room),
                        small,
                        DIM,
                    );
                }
            }
            None => {
                let font = FontId::new(16.0 * u, FontFamily::Proportional);
                let text = if seats.keyboard_seat().is_none() {
                    "Press a button on a controller, or J for the keyboard"
                } else {
                    "Press a button on a controller"
                };
                let text = fit_text(p, text, &font, strip.width() - 20.0 * u);
                p.text(strip.center(), Align2::CENTER_CENTER, text, font, DIM);
            }
        }
    }

    fn paint_bar<I: Copy + PartialEq>(
        &self,
        ui: &mut Ui,
        p: &Painter,
        l: &Layout,
        seats: &Seats<I>,
        go: &str,
        clicks: &mut Vec<Cmd>,
    ) {
        let u = l.u;
        p.rect_filled(l.bar, 0.0, Color32::from_rgba_unmultiplied(4, 6, 11, 230));
        p.hline(l.bar.x_range(), l.bar.top(), Stroke::new(1.0 * u, Color32::from_white_alpha(15)));
        let go = match &self.goes {
            Goes::Link(..) => "Start",
            _ => go,
        };
        let mut hints: Vec<(Chip, &str, Option<Cmd>)> = if self.pad_used {
            vec![
                (Chip::Key("LB RB"), "Move my seat", None),
                (Chip::Y, "Leave seat", Some(Cmd::Leave)),
                (Chip::A, go, Some(Cmd::Go)),
                (Chip::B, "Back", Some(Cmd::Back)),
            ]
        } else {
            let mut h =
                vec![(Chip::Key("Q E"), "Move my seat", None), (Chip::Key("Del"), "Leave seat", Some(Cmd::Leave))];
            if seats.keyboard_seat().is_none() {
                h.push((Chip::Key("J"), "Keyboard joins", None));
            }
            h.push((Chip::Key("Enter"), go, Some(Cmd::Go)));
            h.push((Chip::Key("Esc"), "Back", Some(Cmd::Back)));
            h
        };
        let font = FontId::new(16.0 * u, FontFamily::Proportional);
        let d = theme::CHIP_LARGE;
        let mut x = l.top.left();
        let y = l.bar.center().y;
        for (i, (chip, label, click)) in hints.drain(..).enumerate() {
            let cw = theme::chip_width_at(p, chip, u, d);
            let g = p.layout_no_wrap(label.to_string(), font.clone(), theme::TEXT_2);
            let r = Rect::from_min_size(pos2(x, y - 19.0 * u), vec2(cw + 8.0 * u + g.size().x + 18.0 * u, 38.0 * u));
            if let Some(c) = click {
                let resp = ui.interact(r, Id::unique(("players hint", i)), Sense::click());
                if resp.hovered() {
                    p.rect_filled(r, 10.0 * u, Color32::from_white_alpha(20));
                }
                if resp.clicked() {
                    clicks.push(c);
                }
            }
            theme::paint_chip_at(p, chip, pos2(r.left() + 6.0 * u, y), u, d);
            p.galley(pos2(r.left() + 6.0 * u + cw + 8.0 * u, y - g.size().y / 2.0), g, FG);
            x = r.right() + 8.0 * u;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(game: &str, n: usize, link: bool) -> PlayersScreen {
        let g = romset::games::find(game).unwrap();
        let goes = if link { Goes::Link(game.into(), n) } else { Goes::Play(game.into()) };
        PlayersScreen::new(g, n, link, goes, true)
    }

    /// A frame of the screen, headless: the pad's `navs` and these keys pressed.
    fn frame(
        s: &mut PlayersScreen,
        seats: &mut Seats<u32>,
        ctx: &egui::Context,
        navs: &[Nav],
        keys: &[egui::Key],
    ) -> Option<Outcome> {
        let events = keys
            .iter()
            .map(|&k| egui::Event::Key {
                key: k,
                physical_key: Some(k),
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            })
            .collect();
        run(s, seats, ctx, navs, events)
    }

    fn run(
        s: &mut PlayersScreen,
        seats: &mut Seats<u32>,
        ctx: &egui::Context,
        navs: &[Nav],
        events: Vec<egui::Event>,
    ) -> Option<Outcome> {
        let size = vec2(1280.0, 720.0);
        let input =
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        theme::sync(ctx, size.y);
        let mut out = None;
        let mut full = ctx.run_ui(input, |ui| {
            let view = PlayersView { poster: None, keys: "Arrows, X V Z" };
            out = s.ui(ui.ctx(), navs, seats, &view);
        });
        full.textures_delta.clear();
        out
    }

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        ctx
    }

    fn pad(seats: &mut Seats<u32>, id: u32, name: &str) {
        let key = seats::PadKey { name: name.to_string(), uuid: format!("u{id}") };
        seats.sight(id, key, "Wired".to_string());
    }

    #[test]
    fn the_context_lines_name_the_cabinet_or_the_machines() {
        let vf = screen("vf2", 2, false);
        assert_eq!(vf.context().0, "One cabinet");
        assert!(vf.context().1.starts_with("2 players. P1 plays the left side"));
        assert_eq!((vf.tag(0), vf.tag(1)), ("Left side".to_string(), "Right side".to_string()));
        let dt = screen("daytona", 4, true);
        assert_eq!(dt.context().1, "4 cars on this screen. Each seat drives its own car.");
        assert_eq!(dt.tag(0), "Car 1 · top left");
        assert_eq!(dt.tag(3), "Car 4 · bottom right");
        assert_eq!(screen("daytona", 2, true).tag(1), "Car 2 · right");
    }

    #[test]
    fn the_footer_counts_the_seats() {
        let vf = screen("vf2", 2, false);
        assert_eq!(vf.status(2).0, "Ready.");
        assert_eq!(vf.status(1).0, "1 of 2 seats held. A seat with no controller plays nothing.");
        assert_eq!(vf.status(2).1, "Play");
        let dt = screen("daytona", 4, true);
        assert_eq!(
            dt.status(3),
            ("3 of 4 seats held. Empty seats' cars stay on the grid.".to_string(), "Start, 3 players".to_string())
        );
        assert_eq!(dt.status(1).1, "Start, 1 player");
        assert_eq!(dt.status(4).0, "Everyone is in.");
    }

    #[test]
    fn it_draws_and_a_plays_b_goes_back() {
        let (ctx, mut seats) = (ctx(), Seats::<u32>::default());
        let mut s = screen("vf2", 2, false);
        assert!(frame(&mut s, &mut seats, &ctx, &[], &[]).is_none());
        assert!(
            matches!(frame(&mut s, &mut seats, &ctx, &[Nav::Accept], &[]), Some(Outcome::Go(Goes::Play(g))) if g == "vf2")
        );
        assert!(matches!(frame(&mut s, &mut seats, &ctx, &[Nav::Back], &[]), Some(Outcome::Back)));
        assert!(matches!(frame(&mut s, &mut seats, &ctx, &[], &[egui::Key::Escape]), Some(Outcome::Back)));
        let mut s = screen("daytona", 4, true);
        assert!(matches!(
            frame(&mut s, &mut seats, &ctx, &[], &[egui::Key::Enter]),
            Some(Outcome::Go(Goes::Link(_, 4)))
        ));
    }

    /// LB and RB (Q and E) move the selected seat's player, Y (Delete) has them leave; the
    /// keyboard comes back with J.
    #[test]
    fn a_player_moves_leaves_and_the_keyboard_rejoins() {
        let (ctx, mut seats) = (ctx(), Seats::<u32>::default());
        pad(&mut seats, 1, "Pad");
        seats.join_pad(1, 2);
        let mut s = screen("vf2", 2, false);
        // P1 (the keyboard) selected: RB moves it to P2, and the pad to P1.
        frame(&mut s, &mut seats, &ctx, &[Nav::NextGroup], &[]);
        assert_eq!((seats.held(0), seats.held(1)), (Some(Who::Pad(1)), Some(Who::Keyboard)));
        assert_eq!(s.sel, 1, "the selection follows the player");
        // At the right edge there is no further to go.
        frame(&mut s, &mut seats, &ctx, &[Nav::NextGroup], &[]);
        assert_eq!(seats.keyboard_seat(), Some(1));
        // Q moves it back.
        frame(&mut s, &mut seats, &ctx, &[], &[egui::Key::Q]);
        assert_eq!(seats.keyboard_seat(), Some(0));
        // Y leaves the selected seat: P1, the keyboard.
        frame(&mut s, &mut seats, &ctx, &[Nav::Continue], &[]);
        assert_eq!((seats.held(0), seats.held(1)), (None, Some(Who::Pad(1))));
        // Delete leaves the pad's seat (P2) once it is selected.
        frame(&mut s, &mut seats, &ctx, &[Nav::Right], &[]);
        frame(&mut s, &mut seats, &ctx, &[], &[egui::Key::Delete]);
        assert_eq!(seats.taken(2), 0);
        // J: the keyboard takes the selected seat, if it is free.
        frame(&mut s, &mut seats, &ctx, &[], &[egui::Key::J]);
        assert_eq!(seats.keyboard_seat(), Some(1));
    }

    #[test]
    fn a_click_selects_a_seat_and_an_empty_one_takes_the_keyboard_when_it_is_out() {
        let (ctx, mut seats) = (ctx(), Seats::<u32>::default());
        let mut s = screen("vf2", 2, false);
        frame(&mut s, &mut seats, &ctx, &[], &[]);
        let click = |s: &mut PlayersScreen, seats: &mut Seats<u32>, at: Pos2| {
            run(s, seats, &ctx, &[], vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                let ev = vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ];
                run(s, seats, &ctx, &[], ev);
            }
        };
        click(&mut s, &mut seats, pos2(900.0, 400.0));
        assert_eq!(s.sel, 1);
        // The keyboard leaves P1; a click on the empty P1 seats it again.
        seats.leave(0);
        click(&mut s, &mut seats, pos2(300.0, 400.0));
        assert_eq!(seats.keyboard_seat(), Some(0));
    }
}
