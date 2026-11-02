//! Workstation menus for LAN guests: the screen a guest's use opens runs on
//! the host's menu engine (`horizon_client::menu`), the one single player
//! uses, with vanilla's packets: OpenScreen with the station's `MenuType`,
//! container clicks, `ServerboundContainerButtonClickPacket`, the content and
//! data slot sync after each (`broadcastChanges`), and the inputs back into
//! the inventory when the menu closes (`AbstractContainerMenu.clearContainer`).
use super::{
    Game,
    lan::LanHost,
    lan_crafting::input,
    stations::{Kind as StationKind, Station},
};
use horizon_client::{
    containers::BlockEntity,
    items::ItemStack,
    menu::{AnvilCraft, Kind, Menu},
};
use horizon_core::random::RandomSource;
use horizon_protocol::packets::play::{self, Clientbound as C};

/// A guest's open workstation menu.
pub(super) struct GuestStation {
    pub(super) window: i32,
    state: i32,
    pub(super) kind: StationKind,
    pub(super) pos: [i32; 3],
    menu: Box<Menu>,
    /// The data slots the client was last sent.
    sent_data: Vec<i32>,
    /// The game time of the menu's last sound (`lastSoundTime`).
    last_sound: u64,
}

/// A guest's open lectern: `LecternMenu` over the block entity, made on the
/// menu engine for each click (`Game::lectern_menu`).
pub(super) struct GuestLectern {
    pub(super) window: i32,
    state: i32,
    pub(super) key: (i32, i32, i32),
    /// The book and page the client was last sent (`broadcastChanges`).
    sent: Option<(Option<ItemStack>, i32)>,
}

/// The `MenuType`, the engine's menu and the input slots
/// (`clearContainer`'s container) of a station a guest can use.
pub(super) fn menu_of(kind: StationKind) -> Option<(&'static str, Kind, std::ops::Range<usize>)> {
    Some(match kind {
        StationKind::Loom => ("minecraft:loom", Kind::Loom, 0..3),
        StationKind::Stonecutter => ("minecraft:stonecutter", Kind::Stonecutter, 0..1),
        StationKind::Grindstone => ("minecraft:grindstone", Kind::Grindstone, 0..2),
        StationKind::Anvil => ("minecraft:anvil", Kind::Anvil, 0..2),
        StationKind::Smithing => ("minecraft:smithing", Kind::Smithing, 0..3),
        StationKind::Enchanting => ("minecraft:enchantment", Kind::Enchanting, 0..2),
        StationKind::Beacon => ("minecraft:beacon", Kind::Beacon, 0..1),
        StationKind::Cartography => ("minecraft:cartography_table", Kind::Cartography, 0..2),
        _ => return None,
    })
}

impl Game<'_> {
    /// A workstation a guest's use opened (`ServerPlayer.openMenu`); false
    /// when guests can't use that one yet.
    pub(super) fn lan_open_station(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        station: &Station,
    ) -> bool {
        let Some((menu_type, kind, _)) = menu_of(station.kind) else {
            return false;
        };
        // The mods may stop the menu; the click is spent.
        if !self.guest_may_open(host, conn, menu_type, Some(station.pos)) {
            return true;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(host, conn);
        let title = horizon_protocol::text::translatable(station.kind.title_key(), &[]);
        let Some(window) = host.hub.open_window(conn, menu_type, title) else {
            return true;
        };
        let mut menu = Box::new(Menu::new(kind));
        // Without a mod the take runs AnvilMenu.onTake's order in the click itself.
        menu.anvil_hook = kind == Kind::Anvil && !horizon_client::mod_api::mods().is_empty();
        if let Some(g) = host.hub.guests.iter_mut().find(|g| g.conn == conn)
            && kind == Kind::Enchanting
        {
            // Player.enchantmentSeed, made when first needed.
            if g.xp_seed == 0 {
                g.xp_seed = self.world.random.next_int();
            }
            menu.data[3] = g.xp_seed;
        }
        if kind == Kind::Beacon {
            menu.data = self.beacon_data(host, station.pos).to_vec();
        }
        host.play.entry(conn).or_default().station = Some(GuestStation {
            window,
            state: 0,
            kind: station.kind,
            pos: station.pos,
            menu,
            sent_data: Vec::new(),
            last_sound: 0,
        });
        self.lan_send_station(host, conn);
        true
    }

    /// Runs `op` on the guest's open station (window `window`, any when
    /// None) with their inventory in it, then sends what changed. False when
    /// no such window is open.
    fn lan_station_op(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: Option<i32>,
        op: impl FnOnce(&mut Menu, &horizon_client::menu::Env),
    ) -> bool {
        let Some(mut gs) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.station.take_if(|s| window.is_none_or(|w| s.window == w)))
        else {
            return false;
        };
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
            && !host.hub.guests[gi].spectator()
        {
            // createResult runs when the inputs change (or a rename): a click
            // elsewhere must not hand the hooks their own result again.
            let result_slot = match gs.kind {
                StationKind::Anvil | StationKind::Grindstone => Some(2),
                StationKind::Smithing => Some(3),
                _ => None,
            };
            let before = result_slot.map(|r| {
                let slots: Vec<_> = (0..=r).map(|s| gs.menu.slot(s).cloned()).collect();
                (slots, gs.menu.data[0])
            });
            let mut undo = None;
            let hook = gs.menu.anvil_hook;
            self.guest_menu_op(host, gi, &mut gs.menu, |menu, env| {
                undo = hook.then(|| menu.anvil_undo());
                op(menu, env);
            });
            // AnvilCraftEvent.Pre: the taken result's onTake waits for the mods.
            let mut anvil_craft = gs.menu.anvil_craft.take();
            let mut cancelled = false;
            if let Some(craft) = &anvil_craft {
                let guest = &host.hub.guests[gi];
                let (uuid, name) = (guest.uuid, guest.name.clone());
                let mut cancel = false;
                self.native_anvil_craft_pre(uuid, &name, gs.pos, craft, &mut cancel);
                if cancel && let Some(undo) = undo {
                    // The take is refused: the result, inputs, cost, inventory
                    // and cursor are as before the click (the resync below
                    // tells the client).
                    anvil_craft = None;
                    cancelled = true;
                    self.guest_menu_op(host, gi, &mut gs.menu, |menu, _| menu.anvil_cancel(undo));
                } else {
                    gs.menu.anvil_take(&self.menu_env());
                    let dropped = std::mem::take(&mut gs.menu.dropped);
                    self.drop_guest_items(host, gi, dropped);
                }
            }
            // A cancelled take runs no createResult: the inputs are untouched.
            let recomputed = !cancelled
                && before.is_some_and(|(slots, cost)| {
                    cost != gs.menu.data[0]
                        || slots
                            .iter()
                            .enumerate()
                            .any(|(s, b)| gs.menu.slot(s) != b.as_ref())
                });
            if recomputed && gs.kind == StationKind::Anvil {
                let guest = &host.hub.guests[gi];
                let (uuid, name) = (guest.uuid, guest.name.clone());
                let mut result = gs.menu.slot(2).cloned();
                self.native_prepare_anvil(uuid, &name, gs.pos, &mut result, &mut gs.menu.data[0]);
                let env = self.menu_env();
                gs.menu.set_slot(&env, 2, result);
            }
            if recomputed && gs.kind == StationKind::Grindstone {
                let guest = &host.hub.guests[gi];
                let (uuid, name) = (guest.uuid, guest.name.clone());
                let mut result = gs.menu.slot(2).cloned();
                self.native_prepare_grindstone(uuid, &name, gs.pos, &mut result);
                let env = self.menu_env();
                gs.menu.set_slot(&env, 2, result);
            }
            if recomputed && gs.kind == StationKind::Smithing {
                let guest = &host.hub.guests[gi];
                let (uuid, name) = (guest.uuid, guest.name.clone());
                let mut result = gs.menu.slot(3).cloned();
                self.native_prepare_smithing(uuid, &name, gs.pos, &mut result);
                let env = self.menu_env();
                gs.menu.set_slot(&env, 3, result);
            }
            self.lan_station_sounds(&mut gs);
            if let Some(amount) = gs.menu.ground.take() {
                self.grindstone_taken(gs.pos, amount);
            }
            self.lan_station_combined(host, conn, &mut gs, anvil_craft.as_ref());
            if let Some(button) = gs.menu.enchant_button.take() {
                self.lan_enchant(host, gi, &mut gs, button);
            }
            if std::mem::take(&mut gs.menu.enchant_changed) {
                self.lan_enchant_offers(host, gi, &mut gs);
            }
        }
        host.play.entry(conn).or_default().station = Some(gs);
        self.lan_send_station(host, conn);
        true
    }

    /// How many slots of its own the guest's open station menu has before the
    /// player's inventory (`slot_count - 36`), if one is open.
    pub(super) fn lan_station_own_slots(host: &LanHost, conn: u32) -> Option<usize> {
        let gs = host.play.get(&conn)?.station.as_ref()?;
        Some(gs.menu.slot_count() - 36)
    }

    /// `LecternBlock.openScreen` for a guest: `minecraft:lectern` over the
    /// block entity, with its book and page.
    pub(super) fn lan_open_lectern(&mut self, host: &mut LanHost, conn: u32, key: (i32, i32, i32)) {
        if !self.guest_may_open(host, conn, "minecraft:lectern", Some([key.0, key.1, key.2])) {
            return;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(host, conn);
        let title = horizon_protocol::text::translatable("container.lectern", &[]);
        let Some(window) = host.hub.open_window(conn, "minecraft:lectern", title) else {
            return;
        };
        host.play.entry(conn).or_default().lectern = Some(GuestLectern {
            window,
            state: 0,
            key,
            sent: None,
        });
        self.lan_send_lectern(host, conn, true);
    }

    /// The lectern's book and page for the client: everything (`all`), or
    /// what changed since it was last sent.
    fn lan_send_lectern(&mut self, host: &mut LanHost, conn: u32, all: bool) {
        let Some(gl) = host.play.get_mut(&conn).and_then(|p| p.lectern.as_mut()) else {
            return;
        };
        let (book, page) = match self.world.block_entities.get(&gl.key) {
            Some(BlockEntity::Lectern(l)) => (l.book.clone(), l.page),
            _ => (None, 0),
        };
        let now = Some((book.clone(), page));
        if !all && gl.sent == now {
            return;
        }
        let page_changed = gl.sent.as_ref().is_none_or(|(_, p)| *p != page);
        gl.sent = now;
        gl.state = (gl.state + 1) & 0x7FFF;
        let (window, state) = (gl.window, gl.state);
        let carried = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .and_then(|g| g.carried.clone());
        let content = C::ContainerSetContent(play::ContainerSetContent {
            container: window,
            state,
            items: vec![host.hub.stack(&book)],
            carried: host.hub.stack(&carried),
        });
        host.hub.send_to(conn, &content);
        if all || page_changed {
            host.hub.send_to(
                conn,
                &C::ContainerSetData(play::ContainerSetData {
                    container: window,
                    id: 0,
                    value: page as i16,
                }),
            );
        }
    }

    /// Runs `op` on the guest's lectern menu (window `window`) with their
    /// inventory, the block entity takes the book and page back, and the
    /// client gets what changed. False when no such window is open.
    fn lan_lectern_op(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: i32,
        op: impl FnOnce(&mut Menu, &horizon_client::menu::Env),
    ) -> bool {
        let Some(key) = host
            .play
            .get(&conn)
            .and_then(|p| p.lectern.as_ref())
            .filter(|l| l.window == window)
            .map(|l| l.key)
        else {
            return false;
        };
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
            && !host.hub.guests[gi].spectator()
            && let Some(mut menu) = self.lectern_menu(key)
        {
            // Player.mayBuild: survival and creative.
            let may_build = matches!(host.hub.guests[gi].game_mode, 0 | 1);
            self.guest_menu_op(host, gi, &mut menu, |menu, env| {
                menu.may_build = may_build;
                op(menu, env);
            });
            self.lectern_menu_done(key, &menu);
        }
        self.lan_send_lectern(host, conn, true);
        true
    }

    /// A click on the guest's lectern window; false for other windows.
    pub(super) fn lan_lectern_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        self.lan_lectern_op(host, conn, p.container, |menu, env| {
            let _ = menu.click(env, i32::from(p.slot), i32::from(p.button), input(p.input));
        })
    }

    /// The rest of a taken anvil or smithing result's `onTake`: the guest
    /// pays the levels, the anvil wears (level event 1030 or 1029), the
    /// smithing table sounds (1044). An anvil's `craft` is then reported to
    /// the mods (`AnvilCraftEvent.Post`).
    fn lan_station_combined(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        gs: &mut GuestStation,
        craft: Option<&AnvilCraft>,
    ) {
        let change = std::mem::take(&mut gs.menu.level_change);
        if change != 0 {
            self.as_guest(host, conn, |g| g.give_levels(change));
        }
        if !std::mem::take(&mut gs.menu.combined) {
            return;
        }
        if gs.kind == StationKind::Smithing {
            self.level_event_at(1044, gs.pos, 0);
        } else {
            let creative = gs.menu.creative;
            // A broken anvil closes the menu on the next stillValid check.
            self.anvil_used(gs.pos, creative);
            if let Some(craft) = craft
                && let Some(guest) = host.hub.guests.iter().find(|g| g.conn == conn)
            {
                let (uuid, name) = (guest.uuid, guest.name.clone());
                self.native_anvil_craft_post(uuid, &name, gs.pos, craft);
            }
        }
    }

    /// `EnchantmentMenu.slotsChanged` with a level: the costs and clues for
    /// the guest's seed and the bookshelves around the table.
    fn lan_enchant_offers(&mut self, host: &LanHost, gi: usize, gs: &mut GuestStation) {
        let seed = host.hub.guests[gi].xp_seed;
        let bookcases = self.count_bookshelves(gs.pos);
        let offers = horizon_client::stations::enchant_offers(
            &self.stations().enchantments,
            self.items(),
            seed,
            bookcases,
            gs.menu.slot(0),
        );
        let guest = &host.hub.guests[gi];
        let mut costs = offers.costs;
        self.native_prepare_item_enchant(guest.uuid, &guest.name, gs.pos, &mut costs);
        let mut clues = offers.clues;
        for i in 0..3 {
            // A cost a mod changed shows the clue of the list it will give.
            if costs[i] != offers.costs[i]
                && let Some(stack) = gs.menu.slot(0)
            {
                clues[i] = horizon_client::stations::enchant_clue(
                    &self.stations().enchantments,
                    self.items(),
                    seed,
                    i,
                    costs[i],
                    stack,
                );
            }
        }
        for i in 0..3 {
            gs.menu.data[i] = costs[i];
            let clue = clues[i].as_ref();
            gs.menu.data[4 + i] = clue
                .and_then(|(e, _)| host.hub.registry_id("minecraft:enchantment", e))
                .unwrap_or(-1);
            gs.menu.data[7 + i] = clue.map_or(-1, |(_, l)| *l as i32);
        }
        gs.menu.data[3] = seed;
    }

    /// `EnchantmentMenu.clickMenuButton`'s `access.execute`: the item gets
    /// the offer's enchantments, the guest pays its levels and lapis and
    /// gets a new seed, the table sounds, and the offers are made again.
    fn lan_enchant(&mut self, host: &mut LanHost, gi: usize, gs: &mut GuestStation, button: usize) {
        let Some(item) = gs.menu.slot(0).cloned() else {
            return;
        };
        let g = &host.hub.guests[gi];
        let (seed, creative, conn, uuid, name) =
            (g.xp_seed, g.creative(), g.conn, g.uuid, g.name.clone());
        let Some(enchanted) = horizon_client::stations::apply_enchantment(
            &self.stations().enchantments,
            self.items(),
            seed,
            button,
            gs.menu.data[button],
            &item,
        ) else {
            return;
        };
        let enchanted_for_hook = enchanted.clone();
        let need = button as i32 + 1;
        // Player.onEnchantmentPerformed, and EnchantmentMenu's awardStat(ENCHANT_ITEM).
        self.as_guest(host, conn, |game| {
            game.give_levels(-need);
            game.award_custom_stat("enchant_item", 1);
            // CriteriaTriggers.ENCHANTED_ITEM, with the levels it cost.
            game.trigger_enchanted_item(&enchanted, need);
        });
        host.hub.guests[gi].xp_seed = self.world.random.next_int();
        let env = self.menu_env();
        let lapis = gs.menu.slot(1).cloned().filter(|_| !creative).map(|mut l| {
            l.count = l.count.saturating_sub(need as u32);
            l
        });
        gs.menu.set_slot(&env, 0, Some(enchanted));
        if !creative {
            gs.menu.set_slot(&env, 1, lapis.filter(|l| l.count > 0));
        }
        gs.menu.enchant_changed = false;
        self.lan_enchant_offers(host, gi, gs);
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        let at = gs.pos.map(|v| f64::from(v) + 0.5);
        self.audio.play(
            "minecraft:block.enchantment_table.use",
            Some(at),
            1.0,
            pitch,
        );
        self.native_enchant_item(uuid, &name, gs.pos, &enchanted_for_hook, need);
    }

    /// `ServerGamePacketListenerImpl.handleRenameItem`: the guest's anvil
    /// takes the name (`AnvilMenu.setItemName`).
    pub(super) fn lan_station_rename(&mut self, host: &mut LanHost, conn: u32, name: &str) {
        let anvil = host
            .play
            .get(&conn)
            .and_then(|p| p.station.as_ref())
            .is_some_and(|s| s.kind == StationKind::Anvil);
        if anvil
            && self.chat_filter_text(
                host,
                conn,
                &[name],
                horizon_client::lan::hub::chat_filter::Place::Anvil,
            )
        {
            self.lan_station_op(host, conn, None, |menu, env| {
                menu.rename(env, name);
            });
        }
    }

    /// The sounds the menu played, at the station (heard by everyone around,
    /// as `level.playSound(null, pos, ...)`), once per tick.
    fn lan_station_sounds(&mut self, gs: &mut GuestStation) {
        let sounds = std::mem::take(&mut gs.menu.sounds);
        if sounds.is_empty() || gs.last_sound == self.ticks {
            return;
        }
        gs.last_sound = self.ticks;
        let at = gs.pos.map(|v| f64::from(v) + 0.5);
        for sound in sounds {
            self.audio.play(sound, Some(at), 1.0, 1.0);
        }
    }

    /// A click on the guest's station window; false for other windows.
    pub(super) fn lan_station_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        self.lan_station_op(host, conn, Some(p.container), |menu, env| {
            let _ = menu.click(env, i32::from(p.slot), i32::from(p.button), input(p.input));
        })
    }

    /// `ServerGamePacketListenerImpl.handleContainerButtonClick`:
    /// `clickMenuButton`, then `broadcastChanges` when the menu took it.
    pub(super) fn lan_station_button(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: i32,
        id: i32,
    ) {
        // A mod container's menu: its mod's buttons (`native_menu_button`),
        // as handleContainerButtonClick checks them: the open menu, not a
        // spectator, still valid.
        if let Some(g) = host.hub.guest(conn)
            && let Some(menu) = g.menu.as_ref().filter(|m| m.window == window)
            && let Some(pos) = menu.pos
            && self.mod_menu_at(pos).is_some()
        {
            if !g.spectator() && menu.slots.still_valid(&self.world, g, menu.pos) {
                // The clicking player's cursor stack (`clickMenuButton`'s
                // other half). Single player's own player is a guest of
                // its own host too (`lan.rs:165`), so this one path covers
                // both single player and a LAN guest.
                let mut carried = g.carried.clone();
                // As that player: `native_acting_player`, `native_acting_id`
                // and `native_give_stack` answer for the one who pressed.
                self.as_guest(host, conn, |game| {
                    game.mod_menu_button(pos, id, &mut carried)
                });
                if host.hub.guest(conn).is_some_and(|g| g.carried != carried) {
                    let net = host.hub.stack(&carried);
                    if let Some(g) = host.hub.guest_mut(conn) {
                        g.carried = carried;
                        g.send(&C::SetCursorItem(play::SetCursorItem { item: net }));
                    }
                }
            }
            return;
        }
        // LecternMenu's page buttons and Take Book.
        if self.lan_lectern_op(host, conn, window, |menu, env| {
            menu.button(env, id);
        }) {
            return;
        }
        let identity = host.hub.guest(conn).map(|g| (g.uuid, g.name.clone()));
        let mut selected = false;
        let kind = host
            .play
            .get(&conn)
            .and_then(|p| p.station.as_ref())
            .filter(|s| s.window == window)
            .map(|s| s.kind);
        self.lan_station_op(host, conn, Some(window), |menu, env| {
            selected = menu.button(env, id);
        });
        if selected && let Some((player, name)) = identity {
            match kind {
                Some(StationKind::Loom) => {
                    self.native_player_loom_pattern_select(player, &name, id);
                }
                Some(StationKind::Stonecutter) => {
                    self.native_player_stonecutter_recipe_select(player, &name, id);
                }
                _ => {}
            }
        }
    }

    /// The whole station menu and its changed data slots for the client
    /// (`sendAllDataToRemote` / `broadcastChanges`).
    fn lan_send_station(&mut self, host: &mut LanHost, conn: u32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some(gs) = host.play.get_mut(&conn).and_then(|p| p.station.as_mut()) else {
            return;
        };
        gs.state = (gs.state + 1) & 0x7FFF;
        let own = gs.menu.slot_count() - 36;
        let mut items: Vec<Option<ItemStack>> =
            (0..own).map(|s| gs.menu.slot(s).cloned()).collect();
        let g = &host.hub.guests[gi];
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let carried = g.carried.clone();
        let packet = C::ContainerSetContent(play::ContainerSetContent {
            container: gs.window,
            state: gs.state,
            items: items.iter().map(|s| host.hub.stack(s)).collect(),
            carried: host.hub.stack(&carried),
        });
        gs.sent_data.resize(gs.menu.data.len(), i32::MIN);
        let mut data = Vec::new();
        for (k, &v) in gs.menu.data.iter().enumerate() {
            if gs.sent_data[k] != v {
                gs.sent_data[k] = v;
                data.push(C::ContainerSetData(play::ContainerSetData {
                    container: gs.window,
                    id: k as i16,
                    value: v as i16,
                }));
            }
        }
        host.hub.send_to(conn, &packet);
        host.hub.guests[gi].inventory_told();
        for packet in data {
            host.hub.send_to(conn, &packet);
        }
    }

    /// `AbstractContainerMenu.removed` of the guest's station (`window`;
    /// None closes it whichever it is): the carried stack and the inputs go
    /// back to the inventory, what doesn't fit is dropped. False when no such
    /// window is open.
    pub(super) fn lan_station_closed(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: Option<i32>,
    ) -> bool {
        // A lectern's menu keeps nothing of its own (the book is the block entity's).
        let lectern = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.lectern.take_if(|l| window.is_none_or(|w| l.window == w)));
        if lectern.is_some() {
            // AbstractContainerMenu.removed: the carried stack goes back.
            if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
                && let Some(carried) = host.hub.guests[gi].carried.take()
            {
                self.lan_place_back(host, gi, vec![carried]);
            }
            if window.is_some() {
                return true;
            }
        }
        let Some(gs) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.station.take_if(|s| window.is_none_or(|w| s.window == w)))
        else {
            return false;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return true;
        };
        // AbstractContainerMenu.removed: the carried stack first, then the inputs.
        let mut back: Vec<ItemStack> = host.hub.guests[gi].carried.take().into_iter().collect();
        let inputs = station_inputs(&gs);
        if gs.kind == StationKind::Beacon {
            // BeaconMenu.removed drops the payment (player.drop).
            if !back.is_empty() {
                self.lan_place_back(host, gi, std::mem::take(&mut back));
            }
            self.drop_guest_items(host, gi, inputs);
        } else {
            back.extend(inputs);
        }
        if !back.is_empty() {
            self.lan_place_back(host, gi, back);
        }
        true
    }

    /// `BeaconBlockEntity.dataAccess`: the pyramid's levels and the powers
    /// (`BeaconMenu.encodeEffect`: the `mob_effect` id + 1, 0 for none).
    fn beacon_data(&self, host: &LanHost, pos: [i32; 3]) -> [i32; 3] {
        let Some(BlockEntity::Beacon(b)) = self.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        else {
            return [0; 3];
        };
        let encode = |e: &Option<String>| {
            e.as_deref()
                .and_then(|n| host.hub.registry_id("minecraft:mob_effect", n))
                .map_or(0, |id| id + 1)
        };
        [b.levels, encode(&b.primary), encode(&b.secondary)]
    }

    /// `ServerGamePacketListenerImpl.handleSetBeaconPacket`: the guest's
    /// beacon menu takes the powers (`BeaconMenu.updateEffects`), and they
    /// become the block entity's (`dataAccess.set`: the power_select sound
    /// while the beam is up).
    pub(super) fn lan_station_beacon(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        primary: Option<i32>,
        secondary: Option<i32>,
    ) {
        let Some(pos) = host
            .play
            .get(&conn)
            .and_then(|p| p.station.as_ref())
            .filter(|s| s.kind == StationKind::Beacon)
            .map(|s| s.pos)
        else {
            return;
        };
        let Some((player, name)) = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        // An id the registry doesn't have fails the packet's codec.
        let mut powers: [Option<(i32, String)>; 2] = [None, None];
        for (k, id) in [primary, secondary].into_iter().enumerate() {
            if let Some(id) = id {
                let Some(name) = host.hub.registry_name("minecraft:mob_effect", id) else {
                    return;
                };
                powers[k] = Some((id, name.to_owned()));
            }
        }
        let data = self.beacon_data(host, pos);
        let mut updated = None;
        self.lan_station_op(host, conn, None, |menu, env| {
            menu.data.copy_from_slice(&data);
            let power = |k: usize| powers[k].as_ref().map(|(id, n)| (*id, n.as_str()));
            updated = Some(menu.beacon_update(env, power(0), power(1)));
        });
        // handleSetBeaconPacket: effects the beacon can't give, or no payment, disconnect.
        if updated == Some(false) {
            let name = host
                .hub
                .guests
                .iter()
                .find(|g| g.conn == conn)
                .map(|g| g.name.clone());
            eprintln!(
                "Player {} tried to set invalid beacon effects",
                name.unwrap_or_default()
            );
            host.hub.kick(
                conn,
                horizon_protocol::text::translatable("multiplayer.disconnect.generic", &[]),
            );
        }
        if updated != Some(true) {
            return;
        }
        let [primary, secondary] = powers.map(|p| p.map(|(_, n)| n));
        let mut select_sound = false;
        if let Some(BlockEntity::Beacon(b)) =
            self.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        {
            b.primary = primary.clone();
            b.secondary = secondary.clone();
            select_sound = !b.sections.is_empty();
        }
        self.native_player_change_beacon_effect(
            player,
            &name,
            pos,
            primary.as_deref().unwrap_or(""),
            secondary.as_deref().unwrap_or(""),
        );
        if select_sound {
            self.beacon_sound(pos, "minecraft:block.beacon.power_select");
        }
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
    }

    /// `ServerPlayer.doTick`'s `stillValid`: a guest's station closes once its
    /// block is gone or the guest is out of reach (`isWithinBlockInteractionRange(pos, 4)`).
    pub(super) fn lan_stations_tick(&mut self, host: &mut LanHost) {
        self.lan_lecterns_tick(host);
        let conns: Vec<u32> = host
            .play
            .iter()
            .filter(|(_, p)| p.station.is_some())
            .map(|(c, _)| *c)
            .collect();
        for conn in conns {
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            if !host.hub.here(g) {
                continue;
            }
            let Some(gs) = host.play.get(&conn).and_then(|p| p.station.as_ref()) else {
                continue;
            };
            if !self.lan_station_valid(g, gs) {
                let window = gs.window;
                self.lan_station_closed(host, conn, Some(window));
                host.hub.send_to(
                    conn,
                    &C::ContainerClose(play::ContainerClose { container: window }),
                );
            } else if gs.kind == StationKind::Beacon {
                // The beacon's data slots follow its block entity.
                let data = self.beacon_data(host, gs.pos);
                if let Some(gs) = host.play.get_mut(&conn).and_then(|p| p.station.as_mut())
                    && gs.menu.data[..] != data[..]
                {
                    gs.menu.data.copy_from_slice(&data);
                    self.lan_send_station(host, conn);
                }
            }
        }
    }
}

impl Game<'_> {
    /// A station menu's `stillValid`: its block is still that station and the
    /// guest is within `isWithinBlockInteractionRange(pos, 4)`.
    pub(super) fn lan_station_valid(
        &self,
        g: &horizon_client::lan::guest::Guest,
        gs: &GuestStation,
    ) -> bool {
        let [x, y, z] = gs.pos;
        let name = &self.res.facts().get(self.world.block(x, y, z)).name;
        StationKind::from_block(name) == Some(gs.kind) && g.within_block_range(gs.pos, 4.0)
    }

    /// `LecternMenu.stillValid` (`Container.stillValidBlockEntity`): the block
    /// entity still holds a book and the guest is within the interaction
    /// range and its buffer.
    pub(super) fn lan_lectern_valid(
        &self,
        g: &horizon_client::lan::guest::Guest,
        key: (i32, i32, i32),
    ) -> bool {
        matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Lectern(l)) if l.book.is_some()
        ) && g.within_block_range([key.0, key.1, key.2], 4.0)
    }

    /// The guests' lecterns each tick: `stillValid` (the block entity, its
    /// book, the reach) closes them, and `broadcastChanges` sends a book or
    /// page another player changed.
    fn lan_lecterns_tick(&mut self, host: &mut LanHost) {
        let open: Vec<(u32, i32, (i32, i32, i32))> = host
            .play
            .iter()
            .filter_map(|(c, p)| p.lectern.as_ref().map(|l| (*c, l.window, l.key)))
            .collect();
        for (conn, window, key) in open {
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            if !host.hub.here(g) {
                continue;
            }
            if self.lan_lectern_valid(g, key) {
                self.lan_send_lectern(host, conn, false);
            } else {
                self.lan_station_closed(host, conn, Some(window));
                host.hub.send_to(
                    conn,
                    &C::ContainerClose(play::ContainerClose { container: window }),
                );
            }
        }
    }
}

/// What `clearContainer` gives back from a station's menu.
pub(super) fn station_inputs(gs: &GuestStation) -> Vec<ItemStack> {
    menu_of(gs.kind)
        .map(|(_, _, inputs)| inputs.filter_map(|s| gs.menu.slot(s).cloned()).collect())
        .unwrap_or_default()
}
