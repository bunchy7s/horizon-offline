//! Game modes beyond survival and creative (`GameType`): adventure players
//! cannot build, spectators fly through blocks and only watch.
use super::{Game, Screen};
use horizon_client::command::args::GameMode;

pub(super) fn mode_id_of(mode: GameMode) -> i32 {
    match mode {
        GameMode::Survival => 0,
        GameMode::Creative => 1,
        GameMode::Adventure => 2,
        GameMode::Spectator => 3,
    }
}
pub(super) fn mode_from_id(id: i32) -> GameMode {
    match id {
        1 => GameMode::Creative,
        2 => GameMode::Adventure,
        3 => GameMode::Spectator,
        _ => GameMode::Survival,
    }
}
impl Game<'_> {
    pub(super) fn spectator(&self) -> bool {
        self.mode == GameMode::Spectator
    }
    /// `Abilities.mayBuild`.
    pub(super) fn may_build(&self) -> bool {
        matches!(self.mode, GameMode::Survival | GameMode::Creative)
    }
    pub(super) fn mode_id(&self) -> i32 {
        mode_id_of(self.mode)
    }
    pub(super) fn set_mode_id(&mut self, id: i32) {
        self.set_mode(mode_from_id(id));
    }
    /// `ServerPlayer.setGameMode` and `GameType.updatePlayerAbilities`.
    pub(super) fn set_mode(&mut self, mode: GameMode) {
        let was_spectator = self.spectator();
        // MultiPlayerGameMode.setLocalMode: the mode it leaves.
        if mode != self.mode {
            self.previous_mode = Some(self.mode);
        }
        self.mode = mode;
        self.creative = mode == GameMode::Creative;
        // GameType.updatePlayerAbilities applies at once.
        if let Some(p) = self.player.as_mut() {
            p.invulnerable = mode == GameMode::Creative || mode == GameMode::Spectator;
            p.may_fly = p.invulnerable;
        }
        if mode == GameMode::Spectator {
            self.mining = None;
            self.using = None;
            self.left = false;
            self.dismount_for_spectator();
            if let Some(p) = self.player.as_mut() {
                p.flying = true;
                p.spectator = true;
            }
        } else if let Some(p) = self.player.as_mut() {
            p.spectator = false;
            if was_spectator {
                p.flying_speed = 0.05;
            }
            if !self.creative {
                p.flying = false;
            } else if was_spectator {
                p.flying = true;
            }
        }
        if mode == GameMode::Spectator && self.screen == Screen::Inventory {
            self.screen = Screen::Playing;
        }
    }
}
impl Game<'_> {
    /// `ServerPlayer.setGameMode(SPECTATOR)`: `stopRiding`.
    fn dismount_for_spectator(&mut self) {
        let Some(id) = self.riding else {
            return;
        };
        if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            self.dismount(i);
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            self.dismount_cart(i);
        } else {
            self.riding = None;
        }
    }
}
