//! The spectator menu (`SpectatorGui`): its players and teams from the
//! server's player list and teams, and the teleport request.
use super::Game;
use horizon_client::{
    hud::HudBatch,
    hud_layout, keybinds,
    scoreboard::{comp_text, team_color_rgb},
    spectator_menu::{Member, Outcome, SpectatorMenu, Team},
    tab_list::{SPECTATOR, TabPlayer},
    text::Text,
};
use horizon_protocol::packets::play::{self, Serverbound as S};

impl Game<'_> {
    /// `Util.getMillis` for the menu's fade.
    fn spectator_now(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
    }
    /// `new SpectatorMenu`: the listed players, and the teams with a player
    /// to go to (`TeleportToTeamMenuCategory.createTeamEntries`).
    fn new_spectator_menu(&mut self) -> SpectatorMenu {
        let mut tab: Vec<TabPlayer> = self
            .remote_tab_players()
            .map_or_else(Vec::new, |(players, _, _)| players);
        for p in &mut tab {
            p.skin = self.face_skin(p.uuid, &p.skin);
        }
        let member = |p: &TabPlayer| Member {
            uuid: p.uuid,
            name: p.name.clone(),
            skin: p.skin.clone(),
            enabled: p.game_mode != SPECTATOR,
        };
        let players: Vec<Member> = tab.iter().map(member).collect();
        let teams = self
            .score
            .board
            .teams
            .iter()
            .filter_map(|team| {
                let members: Vec<Member> = team
                    .players
                    .iter()
                    .filter_map(|name| tab.iter().find(|p| &p.name == name))
                    .filter(|p| p.game_mode != SPECTATOR)
                    .map(member)
                    .collect();
                if members.is_empty() {
                    return None;
                }
                // A random member's face is the team's icon.
                let pick = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos() as usize)
                    % members.len();
                Some(Team {
                    name: comp_text(&team.display_name),
                    color: team.color.as_deref().and_then(team_color_rgb),
                    icon: members[pick].skin.clone(),
                    players: members,
                })
            })
            .collect();
        SpectatorMenu::new(players, teams)
    }
    fn spectator_outcome(&mut self, outcome: Outcome) {
        if let Outcome::Teleport(uuid) = outcome {
            // PlayerMenuItem.selectItem.
            self.send(S::TeleportToEntity(play::TeleportToEntity { uuid }));
        }
    }
    /// A hotbar key as a spectator (`SpectatorGui.onHotbarSelected`).
    pub(super) fn spectator_hotbar_key(&mut self, slot: i32) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let outcome = self.spectator_gui.on_hotbar_selected(slot, now, menu);
        self.spectator_outcome(outcome);
    }
    /// `key.spectatorHotbar`: opens the menu, or uses the selected item.
    pub(super) fn spectator_action_key(&mut self) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let outcome = self.spectator_gui.on_action_key(now, menu);
        self.spectator_outcome(outcome);
    }
    /// The menu a press would open, built only when none is up.
    fn spectator_menu_if_needed(&mut self) -> Option<SpectatorMenu> {
        (!self.spectator_gui.is_menu_active()).then(|| self.new_spectator_menu())
    }
    /// `MouseHandler.onScroll` in spectator mode with the menu up.
    pub(super) fn spectator_scroll(&mut self, wheel: i32) {
        let now = self.spectator_now();
        self.spectator_gui.on_mouse_scrolled(wheel, now);
    }
    /// `Hud.extractHotbar` for a spectator, and `extractAction`'s prompt.
    pub(super) fn draw_spectator_hotbar(&mut self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let now = self.spectator_now();
        let labels: Vec<Text> = (1..=9)
            .map(|n| keybinds::display_name(self.settings.keys.get(&format!("key.hotbar.{n}"))))
            .collect();
        let font = &self.gui.atlas.font;
        let mut draws = self
            .spectator_gui
            .hotbar(w, h, now, &|slot| labels[slot].clone(), font);
        draws.extend(self.spectator_gui.action(
            w,
            h,
            now,
            font,
            self.settings.text_background(0.0),
        ));
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
}
