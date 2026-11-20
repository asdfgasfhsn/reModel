//! The launcher's network link play panel (docs/play.md, "Network link
//! play"): the host's lobby (this machine's address and port, the players
//! who have joined, Start) and the joiner's (an address to type, or a host
//! found on the network), then the wait for the start. Everything polled
//! from the launcher's draw; when the session is agreed the page hands it
//! over as `Action::PlayNet`.

use crate::focus::Focus;
use crate::launcher::{Action, Nav};
use crate::menu;
use crate::theme::{self, Chip};
use egui::vec2;
use linknet::{Config, Discovery, Host, HostStatus, Join, JoinStatus};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

pub enum NetState {
    /// Waiting for players; `starting` once Start is pressed.
    Host { host: Host, lan: String, starting: bool },
    /// Choosing a host to join: an address typed, or one found.
    Pick { discovery: Option<Discovery>, addr: String },
    /// Joining a host.
    Join { join: Join, status: String },
    /// Something went wrong; only Back is left.
    Failed(String),
}

pub struct NetPage {
    pub game: &'static romset::Game,
    pub focus: Focus,
    pub cfg: Config,
    pub state: NetState,
    /// Hosts: start by itself when this many have joined (`--net-players`; 0: Start does).
    pub auto: usize,
    /// A message under the heading (a refusal, a mistake in the address).
    pub note: Option<String>,
}

fn play_of(game: &romset::Game) -> &'static romset::LinkPlay {
    crate::link::play_for(game.id).expect("a game that links")
}

/// The master's settings the others start from: the player's own EEPROM image for a
/// game that keeps its settings there.
fn base_for(play: &romset::LinkPlay, game: &romset::Game) -> Option<Vec<u8>> {
    if !matches!(play.settings.store, romset::SettingsStore::Eeprom) {
        return None;
    }
    std::fs::read(crate::settings::data_dir().join(game.id).join("eeprom.bin")).ok().filter(|b| b.len() <= 512)
}

/// An address as typed: `host:port`, or just `host` (the default port).
fn parse_addr(text: &str) -> Result<SocketAddr, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("Type the host's address, like 192.168.1.20:47890".into());
    }
    let with_port = if t.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
        t.to_string()
    } else {
        format!("{t}:{}", linknet::DEFAULT_PORT)
    };
    with_port
        .to_socket_addrs()
        .map_err(|_| format!("Can't make an address of \"{t}\""))?
        .next()
        .ok_or_else(|| format!("No such address: {t}"))
}

impl NetPage {
    pub fn host(game: &'static romset::Game, cfg: Config, host: Host, auto: usize) -> NetPage {
        let lan = linknet::lan_address().map_or_else(|| "this machine's address".to_string(), |i| i.to_string());
        NetPage {
            game,
            focus: Focus::default(),
            cfg,
            state: NetState::Host { host, lan, starting: false },
            auto,
            note: None,
        }
    }

    pub fn pick(game: &'static romset::Game, cfg: Config, addr: String) -> NetPage {
        let discovery = Discovery::broadcast().ok();
        NetPage { game, focus: Focus::default(), cfg, state: NetState::Pick { discovery, addr }, auto: 0, note: None }
    }

    /// Join a host at once (`--join`).
    pub fn join(game: &'static romset::Game, cfg: Config, addr: &str) -> NetPage {
        let mut p = NetPage::pick(game, cfg, addr.to_string());
        p.connect(addr);
        p
    }

    fn connect(&mut self, text: &str) {
        match parse_addr(text).and_then(|a| Join::connect(self.cfg.clone(), a).map_err(|e| e.to_string())) {
            Ok(join) => {
                self.note = None;
                self.state = NetState::Join { join, status: "Connecting".into() };
            }
            Err(e) => self.note = Some(e),
        }
    }

    /// Leave the page: tell the others.
    pub fn close(&mut self) {
        match &mut self.state {
            NetState::Host { host, .. } => host.close(),
            NetState::Join { join, .. } => join.leave(),
            _ => {}
        }
    }

    /// Draw the panel and carry the session on. True when the page should go.
    pub fn paint(&mut self, ctx: &egui::Context, navs: &[Nav], show_focus: bool, actions: &mut Vec<Action>) -> bool {
        ctx.request_repaint_after(Duration::from_millis(40));
        self.focus.show = show_focus;
        let rest = self.focus.begin(navs);
        let mut back = rest.iter().any(|n| matches!(n, Nav::Back));
        let play = play_of(self.game);
        let mut do_start = false;
        let mut do_connect: Option<String> = None;
        let mut ready = None;
        let mut fail = None;
        match &mut self.state {
            NetState::Host { host, starting, .. } => {
                match host.poll() {
                    HostStatus::Ready(s, p) => ready = Some((s, p)),
                    HostStatus::Failed(e) => fail = Some(e),
                    _ => {}
                }
                if !*starting && self.auto > 0 && host.players() >= self.auto {
                    do_start = true;
                }
            }
            NetState::Pick { discovery, .. } => {
                if let Some(d) = discovery {
                    d.poll();
                }
            }
            NetState::Join { join, status } => match join.poll() {
                JoinStatus::Connecting => *status = format!("Connecting to {}", join.host_addr()),
                JoinStatus::Lobby { players } => {
                    *status = format!("Joined. {players} players so far; waiting for the host to start")
                }
                JoinStatus::Params(p) => {
                    let node = p.hosts[join.index()].nodes[0] as usize;
                    join.accept(linknet::settings_hash(play, p.base.as_deref(), node, &[]));
                    *status = "Starting".into();
                }
                JoinStatus::Waiting => *status = "Starting".into(),
                JoinStatus::Ready(s, p) => ready = Some((s, p)),
                JoinStatus::Failed(e) => fail = Some(e),
            },
            NetState::Failed(_) => {}
        }
        if let Some(e) = fail {
            self.state = NetState::Failed(e);
        }
        if let Some((session, params)) = ready {
            actions.push(Action::PlayNet { set: self.game.id.to_string(), session, params: Box::new(params) });
            self.focus.end();
            return true;
        }

        let hints = [(Chip::A, "Choose"), (Chip::B, "Back"), (Chip::Key("Esc"), "Close")];
        let heading = match &self.state {
            NetState::Host { .. } => "Host a network game",
            NetState::Pick { .. } | NetState::Join { .. } | NetState::Failed(_) => "Join a network game",
        };
        let spec = menu::PanelSpec { game: self.game.title, heading, back: true, extra_h: 0.0 };
        let (game, cfg_name) = (self.game, self.cfg.max_hosts);
        let NetPage { focus, state, note, .. } = self;
        let mut found_pick: Option<String> = None;
        let mut state_back = false;
        back |= menu::side_panel(ctx, &spec, &hints, |ui, reg| {
            let s = theme::scale(ui.ctx());
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(24.0 * s, 0.0))), |ui| {
                ui.add_space(12.0 * s);
                let dim = |t: String| egui::RichText::new(t).size(15.0 * s).color(theme::TEXT_2);
                match state {
                    NetState::Host { host, lan, starting } => {
                        ui.label(dim("Tell the other players this address (they choose Join a network game):".into()));
                        ui.add_space(6.0 * s);
                        let port = host.local_addr().port();
                        ui.label(theme::heading(ui.ctx(), &format!("{lan}:{port}"), 30.0));
                        ui.add_space(10.0 * s);
                        ui.label(dim(format!("Players: {} of {}", host.players(), cfg_name)));
                        ui.label(
                            egui::RichText::new("1. You (the master, CAR 1)").size(15.0 * s).color(theme::TEXT),
                        );
                        for (i, (name, rtt)) in host.joiners().iter().enumerate() {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}. {name}   {}",
                                    i + 2,
                                    rtt.map_or(String::new(), |r| format!("{r:.0} ms round trip"))
                                ))
                                .size(15.0 * s)
                                .color(theme::TEXT),
                            );
                        }
                        ui.add_space(8.0 * s);
                        ui.label(
                            egui::RichText::new(format!(
                                "Needs UDP port {port} reachable from the other machines (a firewall may need it opened). \
                                 The delay is picked from the measured round trip: {} frames now.",
                                host.suggest_delay()
                            ))
                            .size(13.0 * s)
                            .color(theme::TEXT_DIM),
                        );
                        ui.add_space(12.0 * s);
                        if *starting {
                            ui.label(dim("Starting...".into()));
                        } else if focus.menu_row(ui, "Start the game", &format!("{} players", host.players())) {
                            if host.players() < 2 {
                                *note = Some("Wait for another player to join first.".into());
                            } else {
                                do_start = true;
                            }
                        }
                    }
                    NetState::Pick { discovery, addr } => {
                        ui.label(dim("The host's address, shown on its screen:".into()));
                        ui.add_space(6.0 * s);
                        let edit = egui::TextEdit::singleline(addr)
                            .hint_text("192.168.1.20:47890")
                            .desired_width(f32::INFINITY)
                            .font(egui::FontId::new(18.0 * s, egui::FontFamily::Proportional));
                        let r = ui.add(edit);
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(8.0 * s);
                        if focus.menu_row(ui, "Connect", "") || enter {
                            do_connect = Some(addr.clone());
                        }
                        ui.add_space(14.0 * s);
                        let found: Vec<linknet::Found> = discovery.as_ref().map(|d| d.found.clone()).unwrap_or_default();
                        let mine: Vec<&linknet::Found> = found.iter().filter(|f| f.game == game.id).collect();
                        ui.label(dim(if mine.is_empty() {
                            "Looking for hosts on this network...".to_string()
                        } else {
                            "Found on this network:".to_string()
                        }));
                        for f in mine {
                            if focus.menu_row(ui, &format!("{}  ({}/{})", f.name, f.players, f.max), &f.addr.to_string()) {
                                found_pick = Some(f.addr.to_string());
                            }
                        }
                    }
                    NetState::Join { status, .. } => {
                        ui.label(dim(status.clone()));
                    }
                    NetState::Failed(e) => {
                        ui.label(egui::RichText::new(e.clone()).size(16.0 * s).color(theme::YELLOW));
                        ui.add_space(12.0 * s);
                        if focus.menu_row(ui, "Back", "") {
                            state_back = true;
                        }
                    }
                }
                if let Some(n) = note.as_ref() {
                    ui.add_space(8.0 * s);
                    ui.label(egui::RichText::new(n.clone()).size(14.0 * s).color(theme::YELLOW));
                }
            });
            false
        });
        self.focus.end();
        if let Some(a) = found_pick {
            do_connect = Some(a);
        }
        if let Some(t) = do_connect {
            self.connect(&t);
        }
        if do_start {
            if let NetState::Host { host, starting, .. } = &mut self.state {
                if !*starting {
                    let base = base_for(play, self.game);
                    match host.start(None, base, &|p, h| {
                        linknet::settings_hash(play, p.base.as_deref(), p.hosts[h].nodes[0] as usize, &[])
                    }) {
                        Ok(()) => *starting = true,
                        Err(e) => self.note = Some(e),
                    }
                }
            }
        }
        if (back || state_back) && !matches!(self.state, NetState::Failed(_)) {
            self.close();
        }
        back || state_back
    }
}
