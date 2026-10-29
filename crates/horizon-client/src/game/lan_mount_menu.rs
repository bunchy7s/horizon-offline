//! The horse inventory for a server's players (`ServerPlayer.openHorseInventory`):
//! shift + use on a tamed horse, donkey, mule, camel, llama or nautilus, or the
//! inventory key while riding one (`ServerboundPlayerCommandPacket`
//! OPEN_INVENTORY), opens `HorseInventoryMenu` over the mount with
//! `ClientboundMountScreenOpenPacket`. Clicks run on the menu engine
//! (`Kind::Mount`) with the mount's saddle, body item and chest copied in and
//! written back, and the slots go to the client after each change, as
//! `broadcastChanges` does.
use super::{Game, lan::LanHost, lan_crafting::input};
use horizon_client::{
    items::ItemStack,
    menu::{Kind, Menu},
};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// A guest's open mount menu.
pub(super) struct GuestMount {
    window: i32,
    mob: u32,
    columns: usize,
    state: i32,
    /// The menu's slots and the carried stack the client was last sent.
    sent: Option<(Vec<Option<ItemStack>>, Option<ItemStack>)>,
}

impl Game<'_> {
    /// `ServerPlayer.openHorseInventory` for mount `mob`: the open menu
    /// closes, the client gets the mount screen and the menu's slots.
    pub(super) fn lan_open_mount(&mut self, host: &mut LanHost, conn: u32, mob: u32) {
        let Some(index) = self.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let columns = super::mount_menu::mount_columns(&self.mobs[index]);
        self.lan_mount_closed(host, conn, None);
        self.lan_station_closed(host, conn, None);
        self.lan_crafting_closed(host, conn, None);
        self.guest_container_closed(host, conn);
        let Some(window) = host.hub.open_mount_window(conn, columns as i32, mob as i32) else {
            return;
        };
        host.play.entry(conn).or_default().mount_window = Some(GuestMount {
            window,
            mob,
            columns,
            state: 0,
            sent: None,
        });
        self.lan_send_mount(host, conn);
    }

    /// `HorseInventoryMenu` over mount `index`, with the player's inventory
    /// loaded by the caller.
    fn mount_engine_menu(&self, index: usize, columns: usize) -> Menu {
        let env = self.menu_env();
        let mut menu = Menu::new(Kind::Mount(columns));
        menu.mount = Some(self.mobs[index].kind.id.to_string());
        let (saddle, body, items) = self.mount_contents(index);
        menu.set_slot(&env, 0, saddle);
        menu.set_slot(&env, 1, body.flatten());
        for (k, item) in items.into_iter().take(columns * 3).enumerate() {
            menu.set_slot(&env, 2 + k, item);
        }
        menu
    }

    /// The menu's slots (the mount's, then the inventory and the hotbar)
    /// and the carried stack, sent when they changed since the last time.
    fn lan_send_mount(&mut self, host: &mut LanHost, conn: u32) {
        let Some(gm) = host.play.get(&conn).and_then(|p| p.mount_window.as_ref()) else {
            return;
        };
        let (mob, columns) = (gm.mob, gm.columns);
        let Some(index) = self.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
            return;
        };
        let (saddle, body, items) = self.mount_contents(index);
        let mut slots = vec![saddle, body.flatten()];
        slots.extend(items.into_iter().take(columns * 3));
        slots.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let carried = g.carried.clone();
        let now = Some((slots, carried));
        let Some(gm) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.mount_window.as_mut())
        else {
            return;
        };
        if gm.sent == now {
            return;
        }
        gm.state = (gm.state + 1) & 0x7FFF;
        let (window, state) = (gm.window, gm.state);
        let (slots, carried) = now.clone().unwrap_or_default();
        gm.sent = now;
        let content = C::ContainerSetContent(play::ContainerSetContent {
            container: window,
            state,
            items: slots.iter().map(|s| host.hub.stack(s)).collect(),
            carried: host.hub.stack(&carried),
        });
        host.hub.send_to(conn, &content);
    }

    /// A click on the guest's mount window; false for other windows.
    pub(super) fn lan_mount_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let Some((mob, columns)) = host
            .play
            .get(&conn)
            .and_then(|pl| pl.mount_window.as_ref())
            .filter(|m| m.window == p.container)
            .map(|m| (m.mob, m.columns))
        else {
            return false;
        };
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
            && !host.hub.guests[gi].spectator()
            && let Some(index) = self.mobs.iter().position(|m| m.id == mob)
        {
            let mut menu = self.mount_engine_menu(index, columns);
            self.guest_menu_op(host, gi, &mut menu, |menu, env| {
                let _ = menu.click(env, i32::from(p.slot), i32::from(p.button), input(p.input));
            });
            self.lan_mount_store(index, columns, &menu);
        }
        // The client is told what the server has, whatever it predicted.
        if let Some(gm) = host
            .play
            .get_mut(&conn)
            .and_then(|pl| pl.mount_window.as_mut())
        {
            gm.sent = None;
        }
        self.lan_send_mount(host, conn);
        true
    }

    /// The engine menu's mount slots back on mount `index`.
    fn lan_store_slots(menu: &Menu, columns: usize) -> Vec<Option<ItemStack>> {
        (0..columns * 3)
            .map(|k| menu.slot(2 + k).cloned())
            .collect()
    }
    fn lan_mount_store(&mut self, index: usize, columns: usize, menu: &Menu) {
        let (_, body, _) = self.mount_contents(index);
        let body = body.map(|_| menu.slot(1).cloned());
        let items = Self::lan_store_slots(menu, columns);
        self.mount_store(index, menu.slot(0).cloned(), body, items);
    }

    /// `AbstractContainerMenu.removed` of the guest's mount menu (`window`;
    /// None closes it whichever it is): the carried stack goes back to the
    /// inventory. False when no such window is open.
    pub(super) fn lan_mount_closed(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: Option<i32>,
    ) -> bool {
        let closed = host
            .play
            .get_mut(&conn)
            .and_then(|p| {
                p.mount_window
                    .take_if(|m| window.is_none_or(|w| m.window == w))
            })
            .is_some();
        if closed
            && let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
            && let Some(carried) = host.hub.guests[gi].carried.take()
        {
            self.lan_place_back(host, gi, vec![carried]);
        }
        closed
    }

    /// Each tick: a mount menu whose mount died or is out of reach closes
    /// (`AbstractMountInventoryMenu.stillValid`: alive, within the entity
    /// interaction range plus 4 of the eyes), and the others follow the
    /// mount's and the inventory's slots.
    pub(super) fn tick_lan_mount_menus(&mut self, host: &mut LanHost) {
        let open: Vec<(u32, u32, i32)> = host
            .play
            .iter()
            .filter_map(|(c, p)| p.mount_window.as_ref().map(|m| (*c, m.mob, m.window)))
            .collect();
        for (conn, mob, window) in open {
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            let eye = [g.position[0], g.position[1] + g.eye_height(), g.position[2]];
            let valid = self.mobs.iter().find(|m| m.id == mob).is_some_and(|m| {
                let (w, h) = (f64::from(m.width()) / 2.0, f64::from(m.height()));
                let p = [m.position.x, m.position.y, m.position.z];
                let lo = [p[0] - w, p[1], p[2] - w];
                let hi = [p[0] + w, p[1] + h, p[2] + w];
                let d: f64 = (0..3)
                    .map(|k| (eye[k] - eye[k].clamp(lo[k], hi[k])).powi(2))
                    .sum();
                // Player.ENTITY_INTERACTION_RANGE 3, plus 4.
                m.alive() && d < 7.0 * 7.0
            });
            if valid {
                self.lan_send_mount(host, conn);
            } else {
                self.lan_mount_closed(host, conn, Some(window));
                host.hub.send_to(
                    conn,
                    &C::ContainerClose(play::ContainerClose { container: window }),
                );
            }
        }
    }
}
