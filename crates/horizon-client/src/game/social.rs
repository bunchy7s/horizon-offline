//! `SocialInteractionsScreen` and `PlayerSocialManager` on the game side: the players Horizon
//! knows (the tab list's), the ones whose chat the player hid, the screen's page, search and
//! scroll, and what hiding does to chat (`Minecraft.isBlocked`). The screen is
//! `social_screen`. Blocking is Mojang's service (`UserApiService.isBlockedPlayer`), which
//! Horizon does not offer: nobody is blocked, as in vanilla's offline mode.
use super::{Game, Screen};
use horizon_client::{
    social_screen::{self, Action, Entry, Page, SocialView},
    tab_list::TabPlayer,
    text::Text,
    ui::EditBox,
};
use std::collections::{HashMap, HashSet};
use winit::{
    event::{ElementState, KeyEvent},
    keyboard::{KeyCode, PhysicalKey},
};

/// `PlayerSocialManager.hiddenPlayers` and the screen's state.
pub(super) struct Social {
    pub hidden: HashSet<u128>,
    /// `discoveredNamesToUUID`, the other way: the names of the players seen, so a hidden player
    /// who left is still listed by name.
    names: HashMap<u128, String>,
    page: Page,
    search: EditBox,
    scroll: f32,
    /// Opened from the pause menu: Done goes back there.
    from_pause: bool,
}

impl Default for Social {
    fn default() -> Self {
        let mut search = EditBox::new(0.0, 0.0, 200.0, 15.0);
        search.max_length = 16;
        Self {
            hidden: HashSet::new(),
            names: HashMap::new(),
            page: Page::All,
            search,
            scroll: 0.0,
            from_pause: false,
        }
    }
}

impl Game<'_> {
    /// The players of the level, as the tab list shows them, and the local player's id.
    fn social_players(&mut self) -> (Vec<TabPlayer>, u128) {
        match self.remote_tab_players() {
            Some((players, _, _)) => {
                let me = self.remote.as_ref().map_or(0, |r| r.profile());
                (players, me)
            }
            None => {
                let uuid = self.level.player_uuid.map_or(0, |u| {
                    u.iter()
                        .fold(0u128, |a, &w| (a << 32) | u128::from(w as u32))
                });
                let mut me = TabPlayer::new(uuid, super::chat::PLAYER_NAME);
                me.skin = "entity/player/wide/steve".into();
                let guests = self.lan.as_ref().map_or(&[][..], |h| &h.hub.guests);
                (super::scoreboard::local_tab_players(me, guests), uuid)
            }
        }
    }
    /// `Minecraft.isBlocked`: whether a player's chat is hidden (never the player's own).
    pub(super) fn social_hides(&mut self, id: u128) -> bool {
        if !self.social.hidden.contains(&id) {
            return false;
        }
        let (_, me) = self.social_players();
        id != me
    }
    /// `new SocialInteractionsScreen(lastScreen)`: it opens on All, with an empty search.
    pub(super) fn open_social(&mut self, from_pause: bool) {
        self.social.page = Page::All;
        self.social.search.set_value("");
        self.social.scroll = 0.0;
        self.social.from_pause = from_pause;
        self.screen = Screen::Social;
        self.capture(false);
    }
    pub(super) fn close_social(&mut self) {
        if self.social.from_pause {
            self.screen = Screen::Paused;
        } else {
            self.screen = Screen::Playing;
            self.capture(true);
        }
    }
    /// The page's rows, sorted and filtered as `SocialInteractionsPlayerList` does.
    fn social_view(&mut self) -> SocialView {
        let (players, me) = self.social_players();
        for p in &players {
            self.social.names.insert(p.uuid, p.name.clone());
        }
        let hidden = self.social.hidden.clone();
        let search = self.social.search.value.to_lowercase();
        let mut entries: Vec<Entry> = Vec::new();
        let online = |id: u128| players.iter().find(|p| p.uuid == id);
        let page = self.social.page;
        let ids: Vec<u128> = match page {
            Page::All => players.iter().map(|p| p.uuid).collect(),
            Page::Hidden => hidden.iter().copied().collect(),
            // `UserApiService.isBlockedPlayer`: nobody, without the service.
            Page::Blocked => Vec::new(),
        };
        for id in ids {
            let (name, skin, removed) = match online(id) {
                Some(p) => (p.name.clone(), p.skin.clone(), false),
                None => (
                    self.social.names.get(&id).cloned().unwrap_or_default(),
                    horizon_client::tab_list::default_skin(id).to_owned(),
                    true,
                ),
            };
            if !search.is_empty() && !name.to_lowercase().contains(&search) {
                continue;
            }
            let skin = self.face_skin(id, &skin);
            entries.push(Entry {
                id,
                name,
                skin,
                hidden: hidden.contains(&id),
                removed,
                local: id == me,
            });
        }
        social_screen::sort_entries(&mut entries);
        let count = players.len();
        let server = self.social_server_name();
        let key = if count > 1 {
            "gui.socialInteractions.server_label.multiple"
        } else {
            "gui.socialInteractions.server_label.single"
        };
        let args = [Text::literal(server), Text::literal(count.to_string())];
        let max = social_screen::max_scroll(self.gui_size().1, entries.len());
        self.social.scroll = self.social.scroll.clamp(0.0, max);
        SocialView {
            page,
            search: self.social.search.value.clone(),
            entries,
            server_label: Text::translate(key, &args),
            scroll: self.social.scroll,
        }
    }
    /// The server's name: a local world's name, a server's entry in the list.
    fn social_server_name(&self) -> String {
        match (&self.integrated, self.server_address()) {
            (Some(server), _) => server.world_name.clone(),
            (None, Some(address)) if self.remote.is_some() => address.to_owned(),
            _ => self.level.name.clone(),
        }
    }
    pub(super) fn draw_social_screen(&mut self, batch: &mut horizon_client::hud::HudBatch) {
        self.flush_skin_slots();
        let view = self.social_view();
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new("social", social_screen::TITLE)
        });
        social_screen::draw(
            batch,
            &self.gui.atlas,
            self.cursor,
            &view,
            &mut self.social.search,
            true,
        );
    }
    pub(super) fn social_click(&mut self) {
        let view = self.social_view();
        let Some(action) = social_screen::click(self.gui_size(), self.cursor, &view) else {
            return;
        };
        self.audio.play_button_click();
        match action {
            Action::Page(page) => {
                self.social.page = page;
                self.social.scroll = 0.0;
            }
            Action::Hide(id) | Action::Show(id) => {
                let hiding = matches!(action, Action::Hide(_));
                let name = self.social.names.get(&id).cloned().unwrap_or_default();
                if hiding {
                    self.social.hidden.insert(id);
                } else {
                    self.social.hidden.remove(&id);
                }
                // `PlayerEntry.onHiddenOrShown`: a line in the chat, and the narrator says it.
                let key = if hiding {
                    "gui.socialInteractions.hidden_in_chat"
                } else {
                    "gui.socialInteractions.shown_in_chat"
                };
                let line = Text::translate(key, &[Text::literal(name)]);
                self.say_system_now(&line);
                self.system_message(line);
            }
            Action::Done => self.close_social(),
        }
    }
    /// `AbstractSelectionList.mouseScrolled`: a row's height and a bit a notch.
    pub(super) fn social_scroll(&mut self, delta: f32) {
        self.social.scroll -= delta * 18.0;
        // `social_view` clamps it to the list.
        let _ = self.social_view();
    }
    /// The search box has the focus from the start (`setInitialFocus`): typing fills it.
    pub(super) fn social_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        match key {
            KeyCode::Escape => self.close_social(),
            _ => {
                // `EditBox.keyPressed`: Backspace, Delete, the arrows, Home and End.
                let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
                if !self.social.search.key(key, ctrl, shift)
                    && self.types_text()
                    && let Some(text) = event.text.as_deref()
                {
                    self.social_type(text);
                }
                self.social.scroll = 0.0;
            }
        }
    }
    pub(super) fn social_backspace(&mut self) {
        self.social.search.key(KeyCode::Backspace, false, false);
        self.social.scroll = 0.0;
    }
    /// Characters into the search box (`searchBox.setMaxLength(16)`).
    pub(super) fn social_type(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.social.search.insert_text(&text);
        self.social.scroll = 0.0;
    }
}
