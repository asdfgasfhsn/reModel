use super::*;
use crate::photo::{saved_notice, Options, Res};

fn options() -> Options {
    Options {
        hud: false,
        hide_base: false,
        shading: None,
        bloom: 0,
        depth: true,
        blur: 1,
        shutter: 0.5,
        res: Res::H(2160),
    }
}

/// Photo mode's dock on its own, in a window of `size` points.
struct Rig {
    ctx: egui::Context,
    m: Mode,
    size: egui::Vec2,
    actions: Vec<Action>,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Rig {
    fn sized(w: f32, h: f32) -> Rig {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut m = Mode::new(options());
        m.free = true;
        m.range = Some((0, 600));
        m.live = 600;
        m.frame = 600;
        m.pad_used = true;
        let mut r = Rig { ctx, m, size: vec2(w, h), actions: Vec::new(), shapes: Vec::new() };
        // A page's widgets register on its first frames.
        r.frame(vec![]).frame(vec![]);
        r
    }

    fn new() -> Rig {
        Rig::sized(1280.0, 720.0)
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> &mut Rig {
        theme::sync(&self.ctx, self.size.y);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            events,
            ..Default::default()
        };
        let picture = Rect::from_min_size(egui::Pos2::ZERO, self.size);
        let Rig { ctx, m, actions, .. } = self;
        let mut out = ctx.run_ui(input, |ui| {
            actions.extend(m.ui(ui.ctx(), picture, "3840×2160 · 1.00x zoom · roll 0°"));
        });
        out.textures_delta.clear();
        self.shapes = out.shapes;
        self
    }

    /// A d-pad (or key) press, then the frame that applies it and one
    /// that lets the focus settle.
    fn press(&mut self, nav: Nav) -> &mut Rig {
        self.m.press(nav);
        self.frame(vec![]).frame(vec![])
    }

    fn presses(&mut self, n: usize, nav: Nav) -> &mut Rig {
        for _ in 0..n {
            self.press(nav);
        }
        self
    }

    /// Every text shape drawn: its text, where, and its smallest font size.
    fn texts(&self) -> Vec<(String, egui::Pos2, f32)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2, f32)>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(t) => {
                    let size = t.galley.job.sections.iter().map(|s| s.format.font_id.size).fold(f32::MAX, f32::min);
                    out.push((t.galley.text().to_string(), t.pos, size));
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        self.shapes.iter().for_each(|c| walk(&c.shape, &mut out));
        out
    }

    fn shows(&self, text: &str) -> bool {
        self.texts().iter().any(|t| t.0 == text)
    }

    fn at(&self, text: &str) -> egui::Pos2 {
        self.texts().into_iter().find(|t| t.0 == text).unwrap_or_else(|| panic!("{text} in {:?}", self.texts())).1
    }

    fn click(&mut self, at: egui::Pos2) -> &mut Rig {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)])
    }

    fn bar_y(&self) -> f32 {
        self.m.dock.top + (PAD_Y + CELLS + GAP + BAR / 2.0) * theme::scale_for(self.size.y)
    }

    /// The pad's focus on the bar's far end (`nav` Left or Right).
    fn go_end(&mut self, nav: Nav) -> &mut Rig {
        let last = self.m.dock.cells.len() - 1;
        self.presses(last, Nav::Down).press(Nav::Down).presses(20, nav)
    }
}

#[test]
fn lb_and_rb_go_round_the_tabs() {
    let mut r = Rig::new();
    assert_eq!(r.m.tab, Tab::Camera);
    r.m.step_tab(1);
    assert_eq!(r.m.tab, Tab::Time);
    for _ in 0..4 {
        r.m.step_tab(1);
    }
    assert_eq!(r.m.tab, Tab::Frame);
    r.m.step_tab(1);
    assert_eq!(r.m.tab, Tab::Camera, "round the ring");
    r.m.step_tab(-1);
    assert_eq!(r.m.tab, Tab::Frame);
    // Each tab shows its own settings, and only those.
    r.m.tab = Tab::Look;
    r.frame(vec![]).frame(vec![]);
    assert!(r.shows("Exposure") && r.shows("Saturation") && r.shows("Tint"));
    assert!(!r.shows("Letterbox") && !r.shows("Field of view"));
    r.m.tab = Tab::Frame;
    r.frame(vec![]).frame(vec![]);
    assert!(r.shows("Letterbox") && r.shows("Photo size") && r.shows("HUD") && !r.shows("Exposure"));
}

#[test]
fn tabs_with_nothing_to_set_are_left_out() {
    let mut m = Mode::new(options());
    // No free camera, no history: Camera and Time have nothing.
    assert_eq!(m.tabs(), [Tab::Focus, Tab::Look, Tab::Effects, Tab::Frame]);
    m.step_tab(1);
    assert_ne!(m.tab, Tab::Camera);
    m.free = true;
    m.range = Some((0, 10));
    assert_eq!(m.tabs().len(), 6);
    // Focus distance, aperture and bokeh wait for depth of field.
    assert_eq!(Tab::Focus.rows().iter().filter(|r| m.shows(**r)).count(), 1);
    m.dof = true;
    assert_eq!(Tab::Focus.rows().iter().filter(|r| m.shows(**r)).count(), 4);
    // Every setting is on exactly one tab.
    for row in [Row::Zoom, Row::Roll, Row::Near, Row::Reset, Row::Frame, Row::Exposure, Row::Resolution, Row::Shading] {
        assert_eq!(TABS.iter().filter(|t| t.rows().contains(&row)).count(), 1, "{row:?}");
    }
}

#[test]
fn the_pad_walks_a_tab_and_changes_values() {
    let mut r = Rig::new();
    assert!(r.m.dock.focus.focused().is_some_and(|id| r.m.dock.cells.first() == Some(&id)), "on the first cell");
    let zoom = r.m.camera.zoom;
    r.press(Nav::Right);
    assert!(r.m.camera.zoom > zoom, "right on Field of view zooms");
    r.press(Nav::Left).press(Nav::Left);
    assert!(r.m.camera.zoom < zoom);
    // Down is the next cell: Roll.
    let roll = r.m.camera.roll;
    r.press(Nav::Down).press(Nav::Right);
    assert!(r.m.camera.roll > roll);
    // Reset is a button: A presses it.
    r.m.camera.zoom = 3.0;
    r.presses(2, Nav::Down);
    assert_eq!(r.m.dock.focus.focused(), r.m.dock.cells.last().copied());
    r.press(Nav::Accept);
    assert_eq!(r.m.camera.zoom, 1.0, "Reset camera");
    assert!(r.actions.is_empty());
    // Down again is the bar; up comes back to the cell it left.
    r.press(Nav::Down);
    assert!(r.m.dock.focus.focused().is_some_and(|id| !r.m.dock.cells.contains(&id)), "on the bar");
    r.press(Nav::Up);
    assert_eq!(r.m.dock.focus.focused(), r.m.dock.cells.last().copied());
    // A on a value takes the photo, as it always did.
    r.press(Nav::Up);
    r.press(Nav::Accept);
    assert!(matches!(r.actions.as_slice(), [Action::Take]), "{:?}", r.actions);
}

#[test]
fn the_bar_is_reached_from_the_pad() {
    // Right along the bar ends on the shutter: A takes the photo.
    let mut r = Rig::new();
    r.go_end(Nav::Right).press(Nav::Accept);
    assert!(matches!(r.actions.as_slice(), [Action::Take]), "the shutter: {:?}", r.actions);
    // The left end is Hide UI.
    let mut r = Rig::new();
    r.go_end(Nav::Left).press(Nav::Accept);
    assert!(!r.m.ui, "Hide UI");
    assert!(r.actions.is_empty());
    // Between them: the tab buttons, and Leave.
    let mut r = Rig::new();
    r.go_end(Nav::Left).presses(3, Nav::Right).press(Nav::Accept);
    assert_eq!(r.m.tab, Tab::Focus, "Hide, Camera, Time, then Focus");
    let mut r = Rig::new();
    r.go_end(Nav::Right).press(Nav::Left).press(Nav::Accept);
    assert!(matches!(r.actions.as_slice(), [Action::Leave]), "{:?}", r.actions);
    let mut r = Rig::new();
    r.go_end(Nav::Right).presses(2, Nav::Left).press(Nav::Accept);
    assert!(r.m.dock.grid, "the thirds grid");
}

#[test]
fn hiding_the_dock_leaves_the_picture() {
    let mut r = Rig::new();
    assert!(r.shows("Camera") && r.shows("Hide (Y)") && r.shows("PHOTO"));
    r.m.ui = false;
    r.frame(vec![]).frame(vec![]);
    assert!(!r.shows("Camera") && !r.shows("Field of view") && !r.shows("PHOTO"));
    assert_eq!(r.m.dock.height, 0.0, "no dock");
    assert!(r.shows("Show controls (Y)"), "a small way back");
    // The way back works with the mouse, and the pad's presses are not kept.
    r.m.press(Nav::Right);
    r.frame(vec![]);
    let at = r.at("Show controls (Y)");
    r.click(at + vec2(20.0, 10.0));
    assert!(r.m.ui);
    r.frame(vec![]);
    assert!(r.shows("Camera"));
    assert_eq!(r.m.camera.zoom, 1.0, "the press made while hidden did nothing");
    // The Hide button hides.
    let at = r.at("Hide (Y)");
    r.click(at + vec2(20.0, 10.0));
    assert!(!r.m.ui);
}

#[test]
fn the_mouse_reaches_everything() {
    let mut r = Rig::new();
    // A tab.
    let at = r.at("Look");
    r.click(at + vec2(10.0, 8.0));
    assert_eq!(r.m.tab, Tab::Look);
    r.frame(vec![]);
    assert!(r.shows("Exposure"));
    // An arrow: Exposure's right arrow, a cell's right edge.
    r.frame(vec![]);
    let s = 1.0;
    let (left, width) = (50.0 + PAD_X, WIDTH - 2.0 * PAD_X);
    let cell = (width - 8.0 * 5.0) / 6.0;
    let arrow = pos2(left + cell - 18.0 * s, r.m.dock.top + (PAD_Y + 30.0) * s);
    let before = r.m.look.exposure;
    r.click(arrow);
    assert!(r.m.look.exposure > before, "{} > {before}", r.m.look.exposure);
    // The shutter and Leave.
    let at = r.at("Leave (B)");
    r.actions.clear();
    r.click(at + vec2(10.0, 8.0));
    assert!(matches!(r.actions.as_slice(), [Action::Leave]));
    r.actions.clear();
    r.click(pos2(left + width - 24.0, r.bar_y()));
    assert!(matches!(r.actions.as_slice(), [Action::Take]), "{:?}", r.actions);
    // The thirds grid.
    let at = r.at("Grid");
    r.click(at + vec2(10.0, 8.0));
    assert!(r.m.dock.grid);
    // A switch toggles from the middle of its cell.
    r.m.tab = Tab::Frame;
    r.frame(vec![]).frame(vec![]);
    let hud = r.at("HUD");
    assert!(!r.m.opts.hud);
    r.click(hud + vec2(40.0, 20.0));
    assert!(r.m.opts.hud);
}

#[test]
fn the_time_strip_scrubs() {
    let mut r = Rig::new();
    r.m.tab = Tab::Time;
    r.frame(vec![]).frame(vec![]);
    assert!(r.shows("Live") && r.texts().iter().any(|t| t.0.starts_with("−10.")), "{:?}", r.texts());
    // From the pad: left steps back a frame.
    r.press(Nav::Left);
    assert_eq!(r.m.wanted, Some(599));
    r.m.wanted = None;
    // From the mouse: halfway along is halfway back.
    let mid = 50.0 + 640.0 - 50.0;
    r.click(pos2(mid, r.m.dock.top + 50.0));
    let at = r.m.wanted.unwrap();
    assert!((280..=320).contains(&at), "{at}");
}

#[test]
fn the_dock_stays_in_the_bottom_strip() {
    const { assert!(FOOTPRINT <= 140.0) };
    for (w, h) in [(1280.0, 720.0), (1920.0, 1080.0), (960.0, 720.0), (2560.0, 1440.0)] {
        let s = theme::scale_for(h);
        let mut r = Rig::sized(w, h);
        for &tab in &TABS {
            r.m.dof = true;
            r.m.tab = tab;
            r.frame(vec![]).frame(vec![]);
            let (top, height) = (r.m.dock.top, r.m.dock.height);
            assert!(h - top <= 140.0 * s + 0.5, "{tab:?} at {h}: the dock starts {} from the bottom", h - top);
            assert!(top + height <= h, "{tab:?} inside the window");
            assert!(height <= 130.0 * s + 0.5, "{tab:?} at {h}: {height} tall");
            // Everything the dock says is inside it (the readout chip aside).
            for (text, pos, _) in r.texts() {
                if pos.y > h / 2.0 {
                    assert!(pos.y >= top - 1.0 && pos.x >= (w - WIDTH * s) / 2.0 - 1.0, "{tab:?}: {text} at {pos:?}");
                }
            }
        }
    }
}

#[test]
fn nothing_is_set_under_14_points() {
    for (w, h) in [(1280.0, 720.0), (1920.0, 1080.0)] {
        let s = theme::scale_for(h);
        let mut r = Rig::sized(w, h);
        r.m.dof = true;
        r.m.opts.blur = 8;
        r.m.dock.grid = true;
        for &tab in &TABS {
            r.m.tab = tab;
            r.frame(vec![]).frame(vec![]);
            let texts = r.texts();
            assert!(texts.len() > 10, "{tab:?}: {texts:?}");
            for (text, _, size) in texts {
                assert!(size >= 14.0 * s - 0.01, "{tab:?}: {text:?} is {size} points at scale {s}");
            }
        }
        // Hidden, and a toast over it.
        r.m.ui = false;
        let mut menu = crate::menu::Menu::default();
        menu.notify("Saved srallyc-1432-08811.png (3840x2160)");
        theme::sync(&r.ctx, h);
        let input =
            egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, r.size)), ..Default::default() };
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut out = r.ctx.run_ui(input.clone(), |ui| {
                menu.photo_notices(ui.ctx());
                r.m.ui(ui.ctx(), Rect::from_min_size(egui::Pos2::ZERO, r.size), "readout");
            });
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        r.shapes = shapes;
        let texts = r.texts();
        assert!(texts.iter().any(|t| t.0.starts_with("Saved srallyc")), "{texts:?}");
        for (text, pos, size) in texts {
            assert!(size >= 14.0 * s - 0.01, "{text:?} is {size} points");
            if text.starts_with("Saved") {
                assert!(pos.y < 100.0 * s, "the toast is at the top, off the dock: {pos:?}");
            }
        }
    }
}

#[test]
fn the_thirds_follow_the_letterbox() {
    let picture = Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 720.0));
    assert_eq!(framed(picture, 0.0), picture);
    let wide = framed(picture, 2.39);
    assert!((wide.width() - 1280.0).abs() < 0.1 && (wide.height() - 1280.0 / 2.39).abs() < 0.1);
    assert_eq!(wide.center(), picture.center());
    let square = framed(picture, 1.0);
    assert!((square.width() - 720.0).abs() < 0.1 && (square.height() - 720.0).abs() < 0.1);
    // The grid is four lines over the picture, on when asked.
    let mut r = Rig::new();
    let lines = |r: &Rig| r.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::LineSegment { .. })).count();
    let none = lines(&r);
    r.m.dock.grid = true;
    r.frame(vec![]);
    assert_eq!(lines(&r), none + 4);
}

#[test]
fn a_photo_saved_is_told_by_its_file_name() {
    let line = "/home/me/Pictures/reModel/srallyc/srallyc-1432-08811.png (3840x2160, drawn at 7680x4320 with 4x MSAA in 41 ms, frame 8811); written in 120 ms";
    assert_eq!(saved_notice(line).as_deref(), Some("Saved srallyc-1432-08811.png (3840x2160)"));
    assert_eq!(saved_notice("/nowhere/x.png: Permission denied (os error 13)"), None);
}
