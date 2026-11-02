//! Controllers: which player each drives, joining with one, and one disconnecting (its player
//! waits for another). One reader for every player's game (`PadRouter`) while splitscreen
//! watches the controllers; otherwise P1 reads them itself, as without splitscreen.
use super::hud::tr;
use super::*;
use crate::game::pad::{PadEvent, PadId};
use horizon_client::gamepad::{Button, PadState};

/// How long a controller's offer to join stays up.
const OFFER: Duration = Duration::from_secs(10);

/// P1's controller, and one P1 gave away (Give Up Pad) while it's offered to join: until the
/// other player is in the world, cancelling, declining or the offer running out gives that same
/// pad back, never another player's.
pub(super) struct P1Pad<Id> {
    pub(super) owner: Option<Id>,
    released: Option<Id>,
}
impl<Id> Default for P1Pad<Id> {
    fn default() -> Self {
        Self {
            owner: None,
            released: None,
        }
    }
}
impl<Id: Copy + Eq> P1Pad<Id> {
    pub(super) fn claim(&mut self, id: Id) {
        self.owner = Some(id);
        if self.released == Some(id) {
            self.released = None;
        }
    }
    fn release(&mut self) -> Option<Id> {
        let id = self.owner.take()?;
        self.released = Some(id);
        Some(id)
    }
    fn cancel(&mut self, id: Id) -> bool {
        if self.released != Some(id) {
            return false;
        }
        self.released = None;
        if self.owner.is_some() {
            return false;
        }
        self.owner = Some(id);
        true
    }
    pub(super) fn joined(&mut self, id: Id) {
        if self.released == Some(id) {
            self.released = None;
        }
    }
    fn gone(&mut self, id: Id) {
        if self.owner == Some(id) {
            self.owner = None;
        }
        self.joined(id);
    }
}

/// A player whose controller disconnected, waiting for one: its seat (1 = P1), and the
/// controller's id and kind (`PadRouter::uuid`) to know it back.
pub(super) struct Lost<Id> {
    pub(super) seat: usize,
    pub(super) id: Id,
    pub(super) uuid: [u8; 16],
}

/// What a button on a controller no player has does while players wait for one.
#[derive(Debug, PartialEq)]
pub(super) enum Claim {
    /// It goes to `lost[k]`.
    Give(usize),
    /// `lost[k]` leaves instead.
    Leave(usize),
    /// Nothing: the others wait (no offer to join meanwhile).
    Wait,
    /// Nobody but P1 waits, and P1 has the keyboard: as if nobody did.
    Pass,
}

/// The same kind of controller back takes back its player with any button; another takes over
/// the player waiting longest with A, or lets it leave with B. P1 is only given its own back
/// (another joins as P1 through the offer to join).
///
/// `uuid` names the model, not the device (gilrs' SDL-style GUID), so it stands for "the same
/// controller" only where a reconnect gets a new id (`model_match`: macOS; elsewhere the id
/// survives and `PadEvent::Back` takes it), and only when one waiting player has that model:
/// otherwise another DualSense would silently take a DualSense player's seat. On Linux gilrs
/// itself reuses a disconnected pad's slot for a new one of the same model, so there a
/// `PadEvent::Back` can be that other controller: gilrs can't tell the two apart.
pub(super) fn claim<Id>(
    lost: &[Lost<Id>],
    uuid: [u8; 16],
    button: Button,
    model_match: bool,
) -> Claim {
    let same_model = || lost.iter().filter(|l| l.uuid == uuid).count() == 1;
    if model_match
        && uuid != [0; 16]
        && same_model()
        && let Some(k) = lost.iter().position(|l| l.uuid == uuid)
    {
        return Claim::Give(k);
    }
    let Some(k) = lost.iter().position(|l| l.seat != 1) else {
        return Claim::Pass;
    };
    match button {
        Button::South => Claim::Give(k),
        Button::East => Claim::Leave(k),
        _ => Claim::Wait,
    }
}

impl Split<'_> {
    /// Give P1 its normal controller reader again and discard pending joins and handoffs.
    pub(super) fn stop_routing(&mut self) {
        if self.host.pad.feed.take().is_some() {
            self.host.pad.release_hardware();
            self.host.split_hud = Default::default();
            self.offers.clear();
            self.fresh.clear();
            self.lost.clear();
            self.p1_pad = P1Pad::default();
            self.pads = PadRouter::default();
        }
    }

    /// The pad's state for its player: nothing while the press that gave it is still held.
    fn pad_state(&mut self, pad: Option<PadId>) -> PadState {
        let Some(id) = pad else {
            return Default::default();
        };
        let state = self.pads.state(id);
        if self.fresh.contains(&id) {
            if pressing(&state) {
                return Default::default();
            }
            self.fresh.retain(|&p| p != id);
        }
        state
    }

    /// Controllers: P1's is the first used in a menu; in a world a controller nobody holds is
    /// offered to join (A), or to drive P1 (Y) while P1 has none. The pads whose players
    /// should join now.
    /// `solo`: no compositor, P1 alone (its controller's output is lent to it after this).
    pub(super) fn route_pads(&mut self, solo: bool) -> Vec<PadId> {
        let now = Instant::now();
        let owner_before = self.p1_pad.owner;
        let offers_before = self.offers.len();
        let mut joins = Vec::new();
        let mut presses = Vec::new();
        for event in self.pads.events() {
            match event {
                PadEvent::Press(id, button) => presses.push((id, button)),
                PadEvent::Gone(id) => self.pad_gone(id),
                PadEvent::Back(id) => {
                    if let Some(k) = self.lost.iter().position(|l| l.id == id) {
                        let seat = self.lost.remove(k).seat;
                        self.give_pad(seat, id);
                    }
                }
            }
        }
        self.forget_lost();
        for (id, button) in presses {
            if Some(id) == self.p1_pad.owner || self.guests.iter().any(|g| g.pad == Some(id)) {
                continue;
            }
            // Pressed, then gone in the same frame.
            if !self.pads.connected(id) {
                continue;
            }
            // In a menu the first controller pressed is P1's, so it can drive the menu, and so
            // it is when P1's own disconnected and its view asks for another. Elsewhere in a
            // world it is offered to join (A joins as the next player, Y gives the pad to P1):
            // the one controller pressed while P1 plays with the keyboard and mouse makes a
            // second player rather than taking P1's controls.
            let in_world = self.in_world();
            // The players waiting for a controller come first: the same kind back takes back
            // its player, another takes over the one waiting longest (`claim`). Only what that
            // passes on, with P1 the one waiting, goes to P1 here.
            if in_world && self.claim_for_lost(id, button) {
                continue;
            }
            let p1_waits = self.lost.iter().any(|l| l.seat == 1);
            let claims_p1 = p1_takes(
                in_world,
                p1_waits,
                self.host.settings.controller.enabled,
                self.p1_pad.owner.is_some(),
            );
            if claims_p1 {
                self.p1_pad.claim(id);
                self.fresh.push(id);
                continue;
            }
            if !in_world {
                continue;
            }
            match self.offers.iter().position(|(o, _)| *o == id) {
                Some(k) => match button {
                    // Two pressing A together both join, in the order they pressed.
                    Button::South => {
                        self.offers.remove(k);
                        joins.push(id);
                    }
                    Button::North
                        if self.p1_pad.owner.is_none() && self.host.settings.controller.enabled =>
                    {
                        self.offers.remove(k);
                        self.p1_pad.claim(id);
                        self.fresh.push(id);
                    }
                    // No: a pad P1 gave up goes back to P1.
                    Button::East => {
                        self.offers.remove(k);
                        self.restore_given_up_pad(id);
                    }
                    _ => {}
                },
                // An offer for each controller while seats are left.
                None if self.players() + joins.len() + self.offers.len() < MAX_PLAYERS => {
                    self.offers.push((id, now));
                    self.ask_to_join();
                }
                None => {}
            }
        }
        let in_world = self.in_world();
        // An offer that ran out: a pad P1 gave up goes back to P1.
        let expired: Vec<_> = self
            .offers
            .iter()
            .filter(|(_, since)| now - *since > OFFER || !in_world)
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.restore_given_up_pad(id);
        }
        self.offers
            .retain(|(_, since)| now - *since <= OFFER && in_world);
        if self.offers.is_empty() {
            self.prompt_join(None);
        } else if self.offers.len() != offers_before
            || self.p1_pad.owner != owner_before
            || !joins.is_empty()
        {
            self.ask_to_join();
        }
        // Each player's glyphs follow its own controller (its game never reads the pads itself).
        if let Some(id) = self.p1_pad.owner {
            self.host.pad.style = self.pads.style(id);
        }
        for g in &mut self.guests {
            if let Some(id) = g.pad {
                g.game.pad.style = self.pads.style(id);
            }
        }
        // P1 alone keeps its controller's output (rumble, the DualSense's triggers): the router
        // lends it its reader for P1's turn (`lend_output`), so no second reader opens. Only
        // when nothing is lent does P1's own reader close: releasing a lent one each frame
        // would stop its rumble a frame after it starts.
        if !lends_output(solo, self.p1_pad.owner.is_some()) {
            self.host.pad.release_hardware();
        }
        self.host.pad.feed = Some(self.pad_state(self.p1_pad.owner));
        for i in 0..self.guests.len() {
            let state = self.pad_state(self.guests[i].pad);
            self.guests[i].game.pad.feed = Some(state);
        }
        joins
    }

    /// P1's Give Up Pad: its controller stops driving P1 and is offered to join, for a friend
    /// to press A (P1 plays on with the keyboard and mouse).
    pub(super) fn give_up_pad(&mut self) {
        let Some(id) = self.p1_pad.release() else {
            return;
        };
        self.offers.retain(|(o, _)| *o != id);
        self.offers.push((id, Instant::now()));
        self.host.pad.release_hardware();
        self.host.pad.feed = Some(PadState::default());
        self.ask_to_join();
    }

    /// A pad P1 gave up whose offer ended without a player in the world: P1's again.
    pub(super) fn restore_given_up_pad(&mut self, id: PadId) {
        if self.pads.connected(id) && self.p1_pad.cancel(id) {
            // The Cancel/No press belongs to the transfer, not P1's menu.
            self.fresh.push(id);
        }
    }

    /// A controller disconnected: its offer and fresh press go; its player waits for one
    /// (P1 keeps the keyboard and mouse).
    fn pad_gone(&mut self, id: PadId) {
        self.fresh.retain(|&p| p != id);
        let offered = self.offers.len();
        self.offers.retain(|(p, _)| *p != id);
        if self.offers.len() != offered && !self.offers.is_empty() {
            self.ask_to_join();
        }
        let uuid = self.pads.uuid(id);
        let owned = self.p1_pad.owner == Some(id);
        self.p1_pad.gone(id);
        if owned {
            if self.in_world() {
                self.lost.push(Lost { seat: 1, id, uuid });
                self.host.split_hud.notice = Some(Notice::new(
                    tr("controller_disconnected", &[]),
                    tr("controller_disconnected.hint", &[]),
                ));
            }
            return;
        }
        let Some(g) = self.guests.iter_mut().find(|g| g.pad == Some(id)) else {
            return;
        };
        g.pad = None;
        let Some(name) = g.name.clone() else {
            // Still picking a profile: as if it pressed Cancel (`pick_profiles`).
            g.game.screen = crate::game::Screen::Playing;
            return;
        };
        self.lost.push(Lost {
            seat: g.seat,
            id,
            uuid,
        });
        g.game.split_hud.prompt = Some(lost_prompt(&name));
    }

    /// Player in `seat` plays with controller `id` (back, or another): its question goes.
    fn give_pad(&mut self, seat: usize, id: PadId) {
        self.fresh.push(id);
        let hud = if seat == 1 {
            self.p1_pad.claim(id);
            &mut self.host.split_hud
        } else {
            let Some(g) = self.guests.iter_mut().find(|g| g.seat == seat) else {
                return;
            };
            g.pad = Some(id);
            &mut g.game.split_hud
        };
        if hud.prompt.as_ref().is_some_and(|p| p.ask == Ask::Lost) {
            hud.prompt = None;
        }
        hud.notice = Some(Notice::new(tr("controller_connected", &[]), ""));
    }

    /// A button on a controller nobody has while players wait for one: true when it answered
    /// them (`claim`).
    fn claim_for_lost(&mut self, id: PadId, button: Button) -> bool {
        match claim(
            &self.lost,
            self.pads.uuid(id),
            button,
            cfg!(target_os = "macos"),
        ) {
            Claim::Give(k) => {
                let seat = self.lost.remove(k).seat;
                self.give_pad(seat, id);
            }
            Claim::Leave(k) => {
                let seat = self.lost.remove(k).seat;
                if let Some(g) = self.guests.iter_mut().find(|g| g.seat == seat) {
                    g.game.split_hud.prompt = None;
                    if let Err(e) = g.game.leave_server(None) {
                        log::warn!("Horizon: splitscreen: {e}");
                    }
                }
            }
            Claim::Wait => {}
            Claim::Pass => return false,
        }
        true
    }

    /// Nobody waits for a player who left or has a controller again (another way), nor for P1
    /// out of a world.
    fn forget_lost(&mut self) {
        let p1 = self.in_world() && self.p1_pad.owner.is_none();
        let guests = &self.guests;
        self.lost.retain(|l| match l.seat {
            1 => p1,
            seat => guests.iter().any(|g| g.seat == seat && g.pad.is_none()),
        });
    }

    /// Every player sees the controllers waiting to join, whatever it is doing.
    fn ask_to_join(&mut self) {
        let title = match self.offers.as_slice() {
            [(id, _)] => tr("join.one", &[&self.pads.name(*id)]),
            offers => tr("join.many", &[&offers.len()]),
        };
        let mut choices = vec![(Button::South, tr("join.yes", &[]))];
        if self.p1_pad.owner.is_none() && self.host.settings.controller.enabled {
            choices.push((Button::North, tr("join.as_p1", &[])));
        }
        choices.push((Button::East, tr("join.no", &[])));
        self.prompt_join(Some(Prompt {
            ask: Ask::Join,
            title,
            choices,
        }));
    }
}

/// Whether a controller nobody holds becomes P1's on its first press: in a menu, where it drives
/// the menus, or in a world while P1's own is gone and P1's view asks for another ("Use
/// another"). Elsewhere in a world it is offered to join, so a controller pressed while P1 plays
/// with the keyboard and mouse starts the join flow instead of taking P1's controls; Y in that
/// offer gives the pad to P1 when it is free. P1 with controllers off plays on the keyboard and
/// mouse: a pad given to it is dead either way.
fn p1_takes(in_world: bool, p1_waits: bool, controllers: bool, p1_has_pad: bool) -> bool {
    (!in_world || p1_waits) && controllers && !p1_has_pad
}

/// Whether P1 gets the split's reader for its turn (`lend_output`): solo, with a controller.
fn lends_output(solo: bool, p1_has_pad: bool) -> bool {
    solo && p1_has_pad
}

/// Still holding the press that gave the pad its player: a button, or a trigger off its rest
/// (gilrs reports a trigger press as a button, but `PadState` keeps it in `triggers`).
fn pressing(state: &PadState) -> bool {
    state.buttons != 0 || state.triggers.iter().any(|&t| t > 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lost(seat: usize, uuid: [u8; 16]) -> Lost<usize> {
        Lost {
            seat,
            id: seat,
            uuid,
        }
    }

    #[test]
    fn a_controller_pressed_in_a_world_is_offered_to_join_not_taken_by_p1() {
        // P1 alone on the keyboard and mouse: the press is offered (A joins, Y gives it to P1).
        assert!(
            !p1_takes(true, false, true, false),
            "a controller pressed in a world never takes P1's controls"
        );
        // P1's own disconnected and its view asks for another: "Use another" hands it over.
        assert!(
            p1_takes(true, true, true, false),
            "a player waiting for a controller takes the next one"
        );
        // The menus still hand the first controller to P1, so it can drive them.
        assert!(
            p1_takes(false, false, true, false),
            "a menu's first controller is P1's"
        );
        assert!(
            !p1_takes(false, false, false, false),
            "controllers off: P1 has none"
        );
        assert!(
            !p1_takes(false, false, true, true),
            "P1 has a controller already"
        );
    }

    #[test]
    fn a_lone_p1_keeps_its_lent_output_between_frames() {
        assert!(
            lends_output(true, true),
            "rumble and triggers survive the next frame"
        );
        assert!(
            !lends_output(true, false),
            "no controller: P1's own reader closes"
        );
        assert!(
            !lends_output(false, true),
            "others play: the router owns every pad"
        );
    }

    #[test]
    fn the_trigger_that_gave_a_pad_is_held_back_too() {
        assert!(!pressing(&PadState::default()));
        let trigger = PadState {
            triggers: [0.0, 0.8],
            ..Default::default()
        };
        assert!(pressing(&trigger), "a held trigger would attack or use");
        let button = PadState {
            buttons: 1,
            ..Default::default()
        };
        assert!(pressing(&button));
    }

    #[test]
    fn the_same_controller_back_takes_its_player_with_any_button() {
        let waiting = [lost(2, [7; 16]), lost(3, [9; 16])];
        assert_eq!(
            claim(&waiting, [9; 16], Button::North, true),
            Claim::Give(1)
        );
    }

    #[test]
    fn another_controller_takes_over_or_lets_go_the_longest_waiting() {
        let waiting = [lost(3, [7; 16]), lost(2, [8; 16])];
        assert_eq!(
            claim(&waiting, [1; 16], Button::South, true),
            Claim::Give(0)
        );
        assert_eq!(
            claim(&waiting, [1; 16], Button::East, true),
            Claim::Leave(0)
        );
        assert_eq!(claim(&waiting, [1; 16], Button::North, true), Claim::Wait);
    }

    #[test]
    fn only_p1_waiting_leaves_the_offer_to_join() {
        let waiting = [lost(1, [7; 16])];
        assert_eq!(claim(&waiting, [1; 16], Button::South, true), Claim::Pass);
        assert_eq!(
            claim(&waiting, [7; 16], Button::South, true),
            Claim::Give(0)
        );
    }

    #[test]
    fn the_same_model_is_only_known_back_where_ids_change_and_unambiguous() {
        // Windows and Linux keep the id across a reconnect: another of the same model asks.
        let waiting = [lost(2, [7; 16])];
        assert_eq!(claim(&waiting, [7; 16], Button::North, false), Claim::Wait);
        assert_eq!(
            claim(&waiting, [7; 16], Button::South, false),
            Claim::Give(0)
        );
        // Two players wait with the same model: no guess, the longest waiting is asked for.
        let waiting = [lost(3, [7; 16]), lost(2, [7; 16])];
        assert_eq!(claim(&waiting, [7; 16], Button::North, true), Claim::Wait);
        assert_eq!(
            claim(&waiting, [7; 16], Button::East, true),
            Claim::Leave(0)
        );
    }

    #[test]
    fn a_controller_the_system_does_not_describe_is_not_known_back() {
        let waiting = [lost(2, [0; 16])];
        assert_eq!(claim(&waiting, [0; 16], Button::North, true), Claim::Wait);
    }

    #[test]
    fn a_pending_transfer_can_be_canceled_without_claiming_an_unrelated_pad() {
        let mut p1 = P1Pad::default();
        p1.claim(7);
        assert_eq!(p1.release(), Some(7));
        assert_eq!(p1.owner, None);
        assert!(!p1.cancel(8));
        assert!(p1.cancel(7));
        assert_eq!(p1.owner, Some(7));
        assert!(!p1.cancel(7));
    }

    #[test]
    fn entering_the_world_commits_the_transfer_and_reclaim_does_not_steal_a_pad() {
        let mut p1 = P1Pad::default();
        p1.claim(7);
        p1.release();
        p1.joined(7);
        assert!(!p1.cancel(7));
        assert_eq!(p1.owner, None);
        p1.claim(7);
        p1.release();
        p1.claim(8);
        assert!(!p1.cancel(7));
        assert_eq!(p1.owner, Some(8));
        p1.release();
        p1.gone(8);
        assert!(!p1.cancel(8));
    }
}
