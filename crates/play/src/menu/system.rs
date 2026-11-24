//! System settings (docs/play.md, "System settings"): what the launcher's
//! Settings (≡) opens. An index of pages (`Menu::system_page`), each the
//! same page a game has (Picture, Display, Motion, Sound, More) but writing
//! the defaults every game starts from, and the pages that belong to no
//! game: Folders (ROM, texture packs, photos), Controllers (the tuning every
//! pad has) and Players (`players`). Each defaults page's footer says which
//! games keep their own values for it.

use super::controls::{response, response_mut, tune, tune_smoothing, Tuned, Tweak};
use super::*;
use crate::settings::PICTURE_KEYS;

/// The folders System settings sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Folder {
    /// Where the games' ROM sets are (`--rom`).
    Roms,
    /// Where texture packs are: `<folder>/<set>/texpack/`.
    Packs,
    /// Where photo mode's photos go: `<folder>/<set>/`.
    Photos,
}

impl Folder {
    const ALL: [Folder; 3] = [Folder::Roms, Folder::Packs, Folder::Photos];

    fn label(self) -> &'static str {
        match self {
            Folder::Roms => "ROM folder",
            Folder::Packs => "Texture packs",
            Folder::Photos => "Photos",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Folder::Roms => "MAME sets, zips or folders. Type the folder's path, then Enter.",
            Folder::Packs => "Each game's pack is <folder>/<game>/texpack. Empty goes back to the default.",
            Folder::Photos => "Each game's photos go in <folder>/<game>. Empty goes back to the default.",
        }
    }
}

/// The pages that list which games keep their own values.
pub const KEEP_PAGES: usize = 6;

/// The settings a page of System settings is, by `KEEP_PAGES` index
/// (Picture, Display, Motion, Sound, More, Controllers): a game has its own
/// values for the page when its layer has any of them.
pub fn page_keys(page: usize) -> Vec<&'static str> {
    match page {
        0 => ["preset"].into_iter().chain(PICTURE_KEYS).collect(),
        1 => vec!["wide", "bezel", "hud_edges", "link_layout"],
        2 => vec!["exact", "smooth", "run_ahead"],
        3 => vec!["sfx_db", "music_db", "link_audio"],
        4 => vec!["geo", "catalogue", "free_play", "net_delay"],
        _ => vec![
            "stick_dead",
            "stick_sens",
            "stick_curve",
            "pedal_dead",
            "pedal_sens",
            "pedal_curve",
            "steer_dead",
            "steer_sens",
            "steer_curve",
            "steer_smooth",
        ],
    }
}

/// What System settings' pages say besides the settings.
#[derive(Clone, Debug, Default)]
pub struct SystemView {
    /// The ROM folder, and how many games are in it.
    pub rom: String,
    pub games: usize,
    /// The texture packs and photos folders chosen, and where they are
    /// without (the default); how many games have a pack folder.
    pub packs: Option<String>,
    pub packs_default: String,
    pub packs_found: usize,
    pub photos: Option<String>,
    pub photos_default: String,
    /// The titles of the games with their own values for each page
    /// (`page_keys` order).
    pub keepers: [Vec<String>; KEEP_PAGES],
}

impl SystemView {
    /// A page's footer: which games keep their own values for it.
    pub fn keepers_note(&self, tab: Tab) -> String {
        keepers_text(
            &self.keepers[match tab {
                Tab::Display => 1,
                Tab::Motion => 2,
                Tab::Sound => 3,
                Tab::More => 4,
            }],
        )
    }

    /// The Picture page's: the same, for the picture.
    pub fn picture_note(&self) -> String {
        keepers_text(&self.keepers[0])
    }

    /// The Controllers page's.
    pub fn controllers_note(&self) -> String {
        keepers_text(&self.keepers[5])
    }

    fn folder_text(&self, f: Folder) -> (String, String) {
        match f {
            Folder::Roms => (self.rom.clone(), format!("{} games found", self.games)),
            Folder::Packs => match &self.packs {
                Some(p) => (p.clone(), format!("{} games have a pack here", self.packs_found)),
                None => (self.packs_default.clone(), "The default".to_string()),
            },
            Folder::Photos => match &self.photos {
                Some(p) => (p.clone(), String::new()),
                None => (self.photos_default.clone(), "The default".to_string()),
            },
        }
    }
}

/// "Top Skater keeps its own values for this page." and the like; with none
/// to name, "Every game uses these."
pub fn keepers_text(titles: &[String]) -> String {
    match titles {
        [] => "Every game uses these.".to_string(),
        [one] => format!("{one} keeps its own values for this page."),
        [a, b] => format!("{a} and {b} keep their own values for this page."),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more keep their own values for this page.", rest.len()),
    }
}

/// What System settings keeps between frames.
#[derive(Default)]
pub(super) struct State {
    index: Focus,
    folders: Focus,
    controllers: Focus,
    pub(super) players: Focus,
    /// A folder's path being typed, and the text field wants the focus.
    edit: Option<(Folder, String)>,
    edit_new: bool,
    /// Opened to quit (Esc on the launcher): the focus goes to Quit reModel.
    pub(super) to_quit: bool,
}

impl State {
    /// A page has just opened.
    pub(super) fn fresh(&mut self) {
        self.folders = Focus::default();
        self.controllers = Focus::default();
        self.players = Focus::default();
        self.edit = None;
    }
}

/// `~/` as the home folder.
fn expand(text: &str) -> String {
    match (text.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).display().to_string(),
        _ => text.to_string(),
    }
}

impl Menu {
    /// A folder's path is being typed: the keys are the text's.
    pub fn editing(&self) -> bool {
        self.sys.edit.is_some()
    }

    /// The index of System settings (in the pause menu's place).
    pub(super) fn system_page(
        &mut self,
        ctx: &egui::Context,
        settings: &GameSettings,
        view: &View,
        navs: &[Nav],
        actions: &mut Vec<Action>,
    ) {
        let focus = &mut self.sys.index;
        focus.show = self.pad_used || self.keys_used;
        for nav in focus.begin(navs) {
            if nav == Nav::Back {
                actions.push(Action::Resume);
            }
        }
        let (picture, sound, speed) = summary(settings);
        let aspect = if settings.wide { "Fit the window" } else { "4:3" };
        let rom = view.system.map(|v| v.rom.clone()).unwrap_or_default();
        let s = theme::scale(ctx);
        let hints = if self.pad_used {
            [(Chip::A, "Open"), (Chip::B, "Close")]
        } else {
            [(Chip::Key("Enter"), "Open"), (Chip::Key("Esc"), "Close")]
        };
        let spec = PanelSpec { game: "reModel", heading: "System settings", back: false, extra_h: 0.0 };
        let mut go = None;
        side_panel(ctx, &spec, &hints, |ui, reg| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(12.0 * s, 0.0))), |ui| {
                focus.scroll_area("system").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0 * s;
                    ui.add_space(8.0 * s);
                    let pages: [(&str, String, Page, Tab); 8] = [
                        ("Folders", short_path(&rom), Page::Folders, Tab::Display),
                        ("Picture", picture.to_string(), Page::Picture, Tab::Display),
                        ("Display", aspect.to_string(), Page::Settings, Tab::Display),
                        ("Motion", speed, Page::Settings, Tab::Motion),
                        ("Sound", sound, Page::Settings, Tab::Sound),
                        ("More", String::new(), Page::Settings, Tab::More),
                        ("Controllers", String::new(), Page::Controllers, Tab::Display),
                        ("Players", String::new(), Page::Players, Tab::Display),
                    ];
                    for (label, value, page, tab) in pages {
                        let hint = if value.is_empty() { "›".to_string() } else { format!("{value} ›") };
                        if focus.menu_row(ui, label, &hint) {
                            go = Some((page, tab));
                        }
                    }
                    ui.add_space(10.0 * s);
                    if focus.menu_row(ui, "Quit reModel", "") {
                        actions.push(Action::Quit);
                    }
                    if self.sys.to_quit {
                        let quit = ui.make_persistent_id(("menu-row", "Quit reModel"));
                        focus.focus_to(quit);
                        self.sys.to_quit = focus.focused() != Some(quit);
                    }
                    ui.add_space(6.0 * s);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0 * s);
                        ui.label(
                            egui::RichText::new(format!("reModel {}", remodel_version::VERSION))
                                .font(FontId::new(14.0 * s, FontFamily::Proportional))
                                .color(theme::TEXT_DIM),
                        );
                    });
                });
            });
            false
        });
        self.sys.index.end();
        if let Some((page, tab)) = go {
            self.go_to(page, tab);
        }
    }

    /// Folders: the ROM, texture pack and photo folders, a row each; A (or a
    /// click) types a new path.
    pub(super) fn folders_page(&mut self, ctx: &egui::Context, view: &View, navs: &[Nav], actions: &mut Vec<Action>) {
        let default = SystemView::default();
        let sv = view.system.unwrap_or(&default);
        let s = theme::scale(ctx);
        let editing = self.sys.edit.is_some();
        let nav_mode = self.pad_used || self.keys_used;
        let focus = &mut self.sys.folders;
        focus.show = nav_mode;
        let mut back = focus.begin(navs).contains(&Nav::Back);
        let esc = editing && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let hints: Vec<(Chip, &str)> = if editing {
            vec![(Chip::Key("Enter"), "Save"), (Chip::Key("Esc"), "Cancel")]
        } else if self.pad_used {
            vec![(Chip::A, "Change"), (Chip::B, "Back")]
        } else {
            vec![(Chip::Key("Enter"), "Change"), (Chip::B, "Back")]
        };
        let spec = PanelSpec { game: "System", heading: "Folders", back: true, extra_h: 96.0 };
        let (mut start, mut done): (Option<Folder>, Option<(Folder, String)>) = (None, None);
        let mut edit = self.sys.edit.take();
        let fresh = std::mem::take(&mut self.sys.edit_new);
        back |= side_panel(ctx, &spec, &hints, |ui, reg| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(12.0 * s, 0.0))), |ui| {
                focus.scroll_area("folders").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0 * s;
                    ui.add_space(8.0 * s);
                    for f in Folder::ALL {
                        let (path, note) = sv.folder_text(f);
                        let sub = if note.is_empty() { path } else { format!("{path} · {note}") };
                        if focus.menu_row2(ui, f.label(), f.label(), &sub) {
                            start = Some(f);
                        }
                        if let Some((at, text)) = edit.as_mut().filter(|e| e.0 == f) {
                            let field = egui::TextEdit::singleline(text)
                                .id(egui::Id::unique("folder-edit"))
                                .font(FontId::new(16.0 * s, FontFamily::Monospace))
                                .desired_width(ui.available_width() - 8.0 * s);
                            let r = ui.add(field);
                            if fresh {
                                r.request_focus();
                            }
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if enter {
                                done = Some((*at, text.clone()));
                            }
                        }
                    }
                });
            });
            let foot = reg.extra.shrink2(vec2(16.0 * s, 8.0 * s));
            ui.painter().hline(reg.extra.x_range(), reg.extra.top(), Stroke::new(1.0, Color32::from_white_alpha(20)));
            ui.scope_builder(egui::UiBuilder::new().max_rect(foot), |ui| {
                let help = match &edit {
                    Some((f, _)) => f.help().to_string(),
                    None => "Changing a folder doesn't move what is in it. reModel looks there from the next \
                             screen on."
                        .to_string(),
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(help)
                            .font(FontId::new(14.0 * s, FontFamily::Proportional))
                            .color(theme::TEXT_DIM),
                    )
                    .wrap(),
                );
            });
            false
        });
        self.sys.folders.end();
        self.sys.edit = edit;
        if let Some(f) = start {
            let text = match f {
                Folder::Roms => sv.rom.clone(),
                Folder::Packs => sv.packs.clone().unwrap_or_default(),
                Folder::Photos => sv.photos.clone().unwrap_or_default(),
            };
            self.sys.edit = Some((f, text));
            self.sys.edit_new = true;
        }
        if let Some((f, text)) = done {
            self.sys.edit = None;
            let text = expand(text.trim());
            match (f, text.is_empty()) {
                (Folder::Roms, true) => {}
                (_, true) => actions.push(Action::SetFolder(f, None)),
                (_, false) => actions.push(Action::SetFolder(f, Some(text))),
            }
        }
        // Back, or Esc, leaves a path being typed before it leaves the page.
        if esc || (back && self.sys.edit.is_some()) {
            self.sys.edit = None;
            back = false;
        }
        if back {
            self.back_to_pause();
        }
    }

    /// Controllers: the tuning every pad has (a stick that aims, a stick
    /// that steers, the triggers), as rows. A game's own is on its Controls
    /// page.
    pub(super) fn controllers_page(
        &mut self,
        ctx: &egui::Context,
        settings: &mut GameSettings,
        view: &View,
        navs: &[Nav],
        _actions: &mut Vec<Action>,
    ) {
        let s = theme::scale(ctx);
        let nav_mode = self.pad_used || self.keys_used;
        let focus = &mut self.sys.controllers;
        focus.show = nav_mode;
        let back = focus.begin(navs).contains(&Nav::Back);
        let mut rows = Rows::new(focus, nav_mode);
        let hints = [(Chip::LeftRight, "Change"), (Chip::B, "Back")];
        let spec = PanelSpec { game: "System", heading: "Controllers", back: true, extra_h: 130.0 };
        let note = view.system.map(|v| v.controllers_note()).unwrap_or_default();
        let mut tweaks: Vec<(Tuned, usize, Tweak)> = Vec::new();
        let back = back
            | side_panel(ctx, &spec, &hints, |ui, reg| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(16.0 * s, 0.0))), |ui| {
                    rows.focus.scroll_area("controllers").show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0 * s;
                        ui.add_space(6.0 * s);
                        let groups = [(Tuned::Steer, "Steering"), (Tuned::Stick, "Stick"), (Tuned::Pedal, "Pedal")];
                        for (kind, name) in groups {
                            let r = response(settings, kind);
                            let names = ["dead zone", "sensitivity", "curve"];
                            let values = [format!("{}%", r.dead), format!("{}%", r.sens), r.curve.label().to_string()];
                            let can = [
                                (r.dead > 0, r.dead < crate::controls::Response::MAX_DEAD),
                                (
                                    r.sens > *crate::controls::Response::SENS.start(),
                                    r.sens < *crate::controls::Response::SENS.end(),
                                ),
                                (
                                    r.curve != crate::controls::Curve::ALL[0],
                                    r.curve != crate::controls::Curve::ALL[crate::controls::Curve::ALL.len() - 1],
                                ),
                            ];
                            for i in 0..3 {
                                let label = format!("{name} {}", names[i]);
                                let step = rows.row(ui, &label, &values[i], can[i], kind.help());
                                if let Some(tw) = tweak(step) {
                                    tweaks.push((kind, i, tw));
                                }
                            }
                            if kind == Tuned::Steer {
                                let m = settings.tuning.smooth;
                                let at = crate::steer::Smoothing::ALL.iter().position(|x| *x == m).unwrap_or(0);
                                let can = (at > 0, at + 1 < crate::steer::Smoothing::ALL.len());
                                let step = rows.row(ui, "Steering smoothing", m.label(), can, kind.help());
                                if let Some(tw) = tweak(step) {
                                    tweaks.push((kind, 3, tw));
                                }
                            }
                        }
                    });
                });
                ui.painter().hline(
                    reg.extra.x_range(),
                    reg.extra.top(),
                    Stroke::new(1.0, Color32::from_white_alpha(20)),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(reg.extra.shrink2(vec2(16.0 * s, 6.0 * s))), |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0 * s;
                    for text in [
                        format!("These apply to every controller. Button layouts are each game's own. {note}"),
                        rows.help(),
                    ] {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(text)
                                    .font(FontId::new(14.0 * s, FontFamily::Proportional))
                                    .color(theme::TEXT_DIM),
                            )
                            .wrap(),
                        );
                    }
                });
                false
            });
        for (kind, row, tw) in tweaks {
            if kind == Tuned::Steer && row == 3 {
                tune_smoothing(&mut settings.tuning.smooth, tw);
            } else {
                tune(response_mut(settings, kind), row, tw);
            }
        }
        self.sys.controllers.end();
        if back {
            self.back_to_pause();
        }
    }
}

/// How a row's step changes a tuning (A cycles a choice, left and right step).
fn tweak(step: Step) -> Option<Tweak> {
    match step {
        Step::None => None,
        Step::Dec => Some(Tweak::By(-1)),
        Step::Inc => Some(Tweak::By(1)),
        Step::Cycle => Some(Tweak::Cycle),
    }
}

/// A path cut from the left to fit a menu row's value.
fn short_path(path: &str) -> String {
    const MAX: usize = 22;
    let n = path.chars().count();
    if n <= MAX {
        return path.to_string();
    }
    format!("…{}", path.chars().skip(n - MAX + 1).collect::<String>())
}
