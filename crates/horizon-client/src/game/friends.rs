//! Play with friends, host side: "Start Sharing" on the Play with Friends
//! screen (`friends_screen`, the pause menu's Friends button) opens the world to LAN if
//! it isn't (with Open to LAN's defaults) and shares that port behind an
//! invite code (horizon-friends). The code holds this computer's addresses
//! and the key, so friends join with no server of ours; a relay the player
//! configured adds a short code and a fallback. The integrated server is the
//! LAN one, unchanged: tunnelled guests are LAN guests that connect from the
//! loopback address.
use super::{Game, lan::LanSettings};
use horizon_client::{
    Result,
    friends_screen::{self, Action, FriendsView},
    hud::HudBatch,
    text::Text,
};
use horizon_friends::{Host, HostConfig, HostEvent, PathMode, host::DEFAULT_PORT, reach, stun};
use horizon_protocol::{GAME_VERSION, PROTOCOL_VERSION};
use std::time::{Duration, Instant};

pub(super) struct Sharing {
    host: Host,
    /// Sharing opened the world to LAN, so stopping closes it again.
    opened_lan: bool,
    code: Option<String>,
    /// Wrong codes tried so far.
    wrong: u32,
    notice: Option<String>,
    copied: Option<Instant>,
    /// Sessions carried directly and through the relay so far.
    paths: (u32, u32),
}

pub const HELP: &[&str] = &[
    "friends host                   share the open world behind an invite code",
    "friends stop                   stop sharing",
    "friends status                 the code, how friends reach this host, the guests",
];

/// `HORIZON_STUN_SERVERS`, else `friendsStun` in horizon.txt, else the defaults.
fn stun_servers() -> Vec<String> {
    if std::env::var_os("HORIZON_STUN_SERVERS").is_some() {
        return stun::servers_from_env();
    }
    let dir = std::env::current_dir().unwrap_or_default();
    match horizon_client::ui_theme::preference(&dir, "friendsStun") {
        Some(list) => stun::parse_servers(&list),
        None => stun::servers_from_env(),
    }
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name).map_or(default, |v| v != "0")
}

impl Game<'_> {
    /// Start sharing the open world.
    pub(super) fn friends_start(&mut self) -> Result<()> {
        if self.friends.is_some() {
            return Ok(());
        }
        // Single player on its server thread shares that server, as Open to
        // LAN publishes it; another server can't be shared.
        if self.title.is_some() || (self.remote.is_some() && self.integrated.is_none()) {
            return Err("open a singleplayer world first".into());
        }
        let opened_lan = self.lan_port().is_none();
        if opened_lan {
            // ShareToLanScreen's defaults, as the world options screen offers them.
            self.lan_open(LanSettings {
                port: horizon_client::lan::available_port(),
                guest_commands: false,
                force_game_mode: true,
            })?;
        }
        let port = self.lan_port().ok_or("the world isn't open to LAN")?;
        let host = match Host::start(HostConfig {
            lan: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            protocol: PROTOCOL_VERSION,
            game: GAME_VERSION.to_owned(),
            relay: horizon_client::net::friends_relay_setting(),
            path: PathMode::from_env(),
            port: std::env::var("HORIZON_FRIENDS_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(DEFAULT_PORT),
            reach: reach::Options {
                // HORIZON_FRIENDS_UPNP=0 leaves the router alone.
                map_port: env_flag("HORIZON_FRIENDS_UPNP", true),
                stun: stun_servers(),
                // Both ends on one machine (tests).
                loopback: env_flag("HORIZON_FRIENDS_LOOPBACK", false),
            },
        }) {
            Ok(h) => h,
            Err(e) => {
                if opened_lan {
                    self.lan_close(true);
                }
                return Err(e.into());
            }
        };
        self.friends = Some(Sharing {
            host,
            opened_lan,
            code: None,
            wrong: 0,
            notice: None,
            copied: None,
            paths: (0, 0),
        });
        Ok(())
    }

    /// Stop sharing: the code ends (direct port closed, router mapping
    /// deleted); the world closes to LAN again if sharing opened it.
    pub(super) fn friends_stop(&mut self) {
        let Some(mut sharing) = self.friends.take() else {
            return;
        };
        sharing.host.stop();
        if sharing.opened_lan {
            self.lan_close(true);
        } else {
            self.system_message(Text::literal("Stopped sharing the invite code."));
        }
    }

    /// The LAN server is closing: the code goes with it.
    pub(super) fn friends_lan_closed(&mut self) {
        if let Some(mut sharing) = self.friends.take() {
            sharing.host.stop();
        }
    }

    /// New codes and sessions, once a tick and while the pause menu is drawn.
    pub(super) fn tick_friends(&mut self) {
        let Some(sharing) = self.friends.as_mut() else {
            return;
        };
        let mut announce = false;
        for event in sharing.host.poll() {
            match event {
                HostEvent::Code(code) => {
                    if sharing.code.is_some() && sharing.wrong >= horizon_friends::host::MAX_WRONG {
                        sharing.notice = Some(
                            "Someone kept trying a wrong short code, so it changed.".to_owned(),
                        );
                        sharing.wrong = 0;
                    }
                    announce = sharing.code.is_none();
                    sharing.code = Some(code);
                }
                HostEvent::WrongCode { count } => {
                    sharing.wrong = count;
                    sharing.notice = Some("Someone tried a wrong code.".to_owned());
                }
                HostEvent::Connected { direct: true, .. } => sharing.paths.0 += 1,
                HostEvent::Connected { direct: false, .. } => sharing.paths.1 += 1,
                HostEvent::Disconnected { .. } => {}
            }
        }
        if announce {
            self.system_message(Text::literal(
                "Your invite code is ready: Game Menu, Play with Friends. Copy it and send it to your friends.",
            ));
        }
    }

    pub(super) fn friends_copy(&mut self) {
        let Some(code) = self.friends.as_ref().and_then(|s| s.code.clone()) else {
            return;
        };
        use horizon_client::clipboard::Clipboard;
        self.clipboard.set(&code);
        if let Some(s) = self.friends.as_mut() {
            s.copied = Some(Instant::now());
        }
    }

    /// Play with Friends. The world is paused under it in singleplayer, so
    /// new codes and sessions are read while it is drawn.
    pub(super) fn draw_friends_screen(&mut self, batch: &mut HudBatch) {
        self.tick_friends();
        let view = self.friends_view();
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new("friends", friends_screen::TITLE)
        });
        friends_screen::draw(batch, &self.gui.atlas, self.cursor, &view, true);
    }
    pub(super) fn friends_screen_click(&mut self) {
        let view = self.friends_view();
        let Some(action) =
            friends_screen::click(&self.gui.atlas, self.gui_size(), self.cursor, &view)
        else {
            return;
        };
        self.audio.play_button_click();
        match action {
            Action::Share => {
                if let Err(e) = self.friends_start() {
                    self.system_message(Text::literal(format!("Couldn't share the world: {e}")));
                }
            }
            Action::Copy => self.friends_copy(),
            Action::Stop => self.friends_stop(),
            // The system browser, as the folder buttons open their folders.
            Action::Help => {
                if let Some(url) = &view.help {
                    super::packs::open_folder(std::path::Path::new(url));
                }
            }
            Action::Done => self.screen = super::Screen::Paused,
        }
    }

    /// What Play with Friends shows.
    pub(super) fn friends_view(&self) -> FriendsView {
        let guests = match &self.lan {
            Some(h) => h.hub.guests.iter().map(|g| g.name.clone()).collect(),
            // On the server thread: the other players the server lists.
            None if self.integrated.is_some() => {
                let me = self.options.player_name();
                let mut names = self.remote_online_names().unwrap_or_default();
                names.retain(|n| *n != me);
                names
            }
            None => Vec::new(),
        };
        let Some(s) = &self.friends else {
            return FriendsView {
                shown: self.remote.is_none() || self.integrated.is_some(),
                ..FriendsView::default()
            };
        };
        let state = s.host.state();
        let mut status = if s.code.is_none() {
            "Finding how friends can reach you…".to_owned()
        } else {
            state.summary.clone()
        };
        if let Some(short) = &state.short {
            status.push_str(&format!(" Short code for the relay: {short}."));
        }
        if let Some(problem) = &state.relay_problem {
            status.push(' ');
            status.push_str(problem);
        }
        let reachable = matches!(
            state.verdict,
            Some(reach::Verdict::Mapped(_) | reach::Verdict::Public)
        ) || state.short.is_some();
        FriendsView {
            shown: true,
            sharing: true,
            code: s.code.clone(),
            status,
            reachable,
            help: (!reachable && s.code.is_some()).then(|| reach::HELP_URL.to_owned()),
            guests,
            notice: s.notice.clone(),
            copied: s
                .copied
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2)),
        }
    }

    /// `horizon-ctl friends ...`.
    pub(super) fn friends_control(&mut self, words: &[&str]) -> Result<Option<serde_json::Value>> {
        use serde_json::json;
        if words[0] != "friends" {
            return Ok(None);
        }
        match words.get(1).copied() {
            Some("host") => {
                self.friends_start()?;
                Ok(Some(json!({ "ok": true, "lan_port": self.lan_port() })))
            }
            Some("stop") => {
                self.friends_stop();
                Ok(Some(json!({ "ok": true })))
            }
            Some("status") => {
                self.tick_friends();
                let view = self.friends_view();
                let state = self.friends.as_ref().map(|s| s.host.state());
                Ok(Some(json!({
                    "sharing": view.sharing,
                    "code": view.code,
                    "short": state.as_ref().and_then(|s| s.short.as_ref().map(ToString::to_string)),
                    "summary": view.status,
                    "reachable": view.reachable,
                    "verdict": state.as_ref().and_then(|s| s.verdict.as_ref().map(|v| format!("{v:?}"))),
                    "direct_port": state.as_ref().map(|s| s.port),
                    "sessions": self.friends.as_ref().map_or(0, |s| s.host.sessions()),
                    "direct": self.friends.as_ref().map_or(0, |s| s.paths.0),
                    "relayed": self.friends.as_ref().map_or(0, |s| s.paths.1),
                    "guests": view.guests,
                    "lan_port": self.lan_port(),
                })))
            }
            _ => Err("friends host|stop|status".into()),
        }
    }
}
