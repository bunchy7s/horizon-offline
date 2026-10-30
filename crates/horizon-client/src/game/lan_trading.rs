//! Trading for LAN guests: the villager screen a guest opens runs the same
//! merchant menu as the host's (`horizon_client::menu`, `Kind::Merchant`),
//! with vanilla's packets: OpenScreen (`minecraft:merchant`), MerchantOffers,
//! the select trade packet, container clicks and the content sync after each.
use super::{
    Game, lan::LanHost, lan_actions::guest_within_entity_range, stations::Kind as StationKind,
    trading::MerchantState,
};
use horizon_client::{
    items::ItemStack,
    menu::{Env, Input, Kind, Menu},
    merchant,
};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// A guest's open trading screen.
pub(super) struct GuestMerchant {
    pub(super) window: i32,
    state: i32,
    /// The payment slots and the result.
    slots: [Option<ItemStack>; 3],
    pub(super) merchant: Box<MerchantState>,
    /// The offers the client was last sent.
    sent: Vec<merchant::MerchantOffer>,
    sent_level: i32,
}

/// A guest's UUID as gossip targets it.
fn uuid_words(uuid: u128) -> [i32; 4] {
    [
        (uuid >> 96) as i32,
        (uuid >> 64) as i32,
        (uuid >> 32) as i32,
        uuid as i32,
    ]
}

impl Game<'_> {
    /// A trading screen the guest's interaction opened (it opened as a
    /// station while the guest acted): handed to the guest.
    pub(super) fn lan_open_merchant(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        mut state: MerchantState,
    ) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let uuid = uuid_words(host.hub.guests[gi].uuid);
        if let Some(id) = state.mob
            && state.session.villager
        {
            // startTrading with the guest's reputation. The session already
            // captured their Hero effect while they acted as the player.
            state.session.reputation = self.villager_reputation_of(id, uuid);
            state.session.update_special_prices();
        }
        let title = self.merchant_display_name(&state);
        // The container or workstation the trading screen replaces is closed.
        self.guest_container_closed(host, conn);
        self.lan_station_closed(host, conn, None);
        let Some(window) = host.hub.open_merchant_window(conn, title) else {
            self.merchant_closed(&mut state);
            return;
        };
        let mut gm = GuestMerchant {
            window,
            state: 0,
            slots: [None, None, None],
            merchant: Box::new(state),
            sent: Vec::new(),
            sent_level: 0,
        };
        self.lan_send_offers(host, conn, &mut gm, true);
        self.lan_send_merchant_content(host, conn, &mut gm);
        host.play.entry(conn).or_default().merchant = Some(gm);
    }
    /// `Entity.getDisplayName` of the merchant, the screen's title: its name
    /// (custom, else the profession's) with the show_entity hover event and
    /// its UUID as insertion.
    fn merchant_display_name(&mut self, state: &MerchantState) -> horizon_world::nbt::Tag {
        use horizon_world::nbt::Tag;
        let s = &state.session;
        let (kind, key) = if s.villager {
            let p = s.profession.trim_start_matches("minecraft:");
            (
                "minecraft:villager",
                format!("entity.minecraft.villager.{p}"),
            )
        } else {
            (
                "minecraft:wandering_trader",
                "entity.minecraft.wandering_trader".to_owned(),
            )
        };
        let name = match &state.name {
            Some(n) => horizon_protocol::text::literal(n),
            None => Tag::compound([("translate", Tag::String(key.as_str().into()))]),
        };
        let uuid = state.mob.map_or([0; 4], |id| self.mob_uuid(id));
        let w = uuid.map(|v| v as u32);
        let text = format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:04x}{:08x}",
            w[0],
            w[1] >> 16,
            w[1] & 0xffff,
            w[2] >> 16,
            w[2] & 0xffff,
            w[3]
        );
        let mut out = match name.clone() {
            Tag::Compound(c) => c,
            other => {
                let mut c = horizon_world::nbt::Compound::new();
                c.insert("text".into(), other);
                c
            }
        };
        out.insert(
            "hover_event".into(),
            Tag::compound([
                ("action", Tag::String("show_entity".into())),
                ("id", Tag::String(kind.into())),
                ("uuid", Tag::IntArray(uuid.to_vec())),
                ("name", name),
            ]),
        );
        out.insert("insertion".into(), Tag::String(text.as_str().into()));
        Tag::Compound(out)
    }
    /// The screen title: the custom name, else the profession (the client
    /// adds the level itself).
    /// `ServerPlayer.sendMerchantOffers`, when the offers changed.
    fn lan_send_offers(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        gm: &mut GuestMerchant,
        force: bool,
    ) {
        let s = &gm.merchant.session;
        if !force && s.offers == gm.sent && s.level == gm.sent_level {
            return;
        }
        let offers: Vec<play::NetOffer> = s
            .offers
            .iter()
            .filter_map(|o| {
                let cost = |st: &ItemStack, hub: &mut horizon_client::lan::hub::Hub| {
                    hub.stack(&Some(st.clone())).map(play::NetCost::from)
                };
                Some(play::NetOffer {
                    cost_a: cost(&o.base_cost_a.stack, &mut host.hub)?,
                    result: host.hub.stack(&Some(o.result.clone()))?,
                    cost_b: match &o.cost_b {
                        Some(b) => Some(cost(&b.stack, &mut host.hub)?),
                        None => None,
                    },
                    out_of_stock: o.out_of_stock(),
                    uses: o.uses,
                    max_uses: o.max_uses,
                    xp: o.xp,
                    special_price: o.special_price_diff,
                    price_multiplier: o.price_multiplier,
                    demand: o.demand,
                })
            })
            .collect();
        host.hub.send_to(
            conn,
            &C::MerchantOffers(play::MerchantOffers {
                container: gm.window,
                offers,
                level: s.level,
                xp: s.xp,
                show_progress: s.villager,
                can_restock: s.villager,
            }),
        );
        gm.sent = s.offers.clone();
        gm.sent_level = s.level;
    }
    fn lan_send_merchant_content(&mut self, host: &mut LanHost, conn: u32, gm: &mut GuestMerchant) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let mut items: Vec<Option<ItemStack>> = gm.slots.to_vec();
        let g = &host.hub.guests[gi];
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let carried = g.carried.clone();
        let net: Vec<_> = items.iter().map(|s| host.hub.stack(s)).collect();
        let carried = host.hub.stack(&carried);
        gm.state = (gm.state + 1) & 0x7FFF;
        host.hub.send_to(
            conn,
            &C::ContainerSetContent(play::ContainerSetContent {
                container: gm.window,
                state: gm.state,
                items: net,
                carried,
            }),
        );
    }
    /// Runs `op` on the guest's merchant menu (their inventory copied in and
    /// back), then syncs the client. False when the guest has no such window.
    fn lan_merchant_op(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: Option<i32>,
        op: impl FnOnce(&mut Menu, &Env),
    ) -> bool {
        let Some(mut gm) = host.play.get_mut(&conn).and_then(|p| p.merchant.take()) else {
            return false;
        };
        if window.is_some_and(|w| w != gm.window) {
            host.play.entry(conn).or_default().merchant = Some(gm);
            return false;
        }
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return false;
        };
        let mut session = gm.merchant.session.clone();
        self.merchant_sync_in(&mut session, gm.merchant.mob);
        let next_map = self.peek_map_id();
        let tag = |t: &str| self.structure_tag(t);
        let maps = super::trading::GameMaps {
            terrain: self.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let trade_env = merchant::TradeEnv {
            trades: &self.stations().trades,
            enchantments: &self.stations().enchantments,
            maps: Some(&maps),
        };
        let env = Env {
            items: self.items(),
            recipes: self.recipes(),
            brewing: &self.stations().brewing,
            banners: &self.stations().banners,
            station_recipes: &self.stations().recipes,
            enchantments: &self.stations().enchantments,
            names: &self.pack.names,
            trade: Some(&trade_env),
            maps: None,
        };
        let mut menu = Menu::new(Kind::Merchant);
        {
            let g = &host.hub.guests[gi];
            for i in 0..36 {
                menu.set_player_item(i, g.inventory[i].clone());
            }
            menu.set_player_item(40, g.offhand.clone());
            menu.selected = g.selected;
            menu.creative = g.creative();
            menu.carried = g.carried.clone();
        }
        for (i, s) in gm.slots.iter().enumerate() {
            menu.set_slot(&env, i, s.clone());
        }
        menu.merchant = Some(session);
        op(&mut menu, &env);
        let made = maps.made.take();
        let session = menu.merchant.take().expect("attached");
        let dropped = std::mem::take(&mut menu.dropped);
        gm.slots = [
            menu.slot(0).cloned(),
            menu.slot(1).cloned(),
            menu.slot(2).cloned(),
        ];
        {
            let g = &mut host.hub.guests[gi];
            for i in 0..36 {
                g.inventory[i] = menu.player_item(i).cloned();
            }
            g.offhand = menu.player_item(40).cloned();
            g.carried = menu.carried.take();
        }
        self.register_exploration_maps(made);
        let (session, traded) = self.merchant_apply(session, gm.merchant.mob);
        gm.merchant.session = session;
        if !traded.is_empty() {
            // The guest's own statistic and advancements.
            self.as_guest(host, conn, |g| g.merchant_traded(gm.merchant.mob, &traded));
        }
        self.drop_guest_items(host, gi, dropped);
        self.lan_send_offers(host, conn, &mut gm, false);
        self.lan_send_merchant_content(host, conn, &mut gm);
        host.play.entry(conn).or_default().merchant = Some(gm);
        true
    }
    /// A container click on the guest's trading window.
    pub(super) fn lan_merchant_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        let input = match p.input {
            0 => Input::Pickup,
            1 => Input::QuickMove,
            2 => Input::Swap,
            3 => Input::Clone,
            4 => Input::Throw,
            5 => Input::QuickCraft,
            _ => Input::PickupAll,
        };
        let (slot, button) = (i32::from(p.slot), i32::from(p.button));
        self.lan_merchant_op(host, conn, Some(p.container), |menu, env| {
            let _ = menu.click(env, slot, button, input);
        })
    }
    /// ServerboundSelectTradePacket: the selection hint, then tryMoveItems.
    pub(super) fn lan_select_trade(&mut self, host: &mut LanHost, conn: u32, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        self.lan_merchant_op(host, conn, None, |menu, env| {
            menu.merchant_select(env, index)
        });
    }
    /// The guest closed the trading window (or it is closed for them):
    /// MerchantMenu.removed, then the villager gets its state back.
    pub(super) fn lan_merchant_closed(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: Option<i32>,
    ) -> bool {
        if !self.lan_merchant_op(host, conn, window, |menu, env| menu.merchant_close(env)) {
            return false;
        }
        let Some(mut gm) = host.play.get_mut(&conn).and_then(|p| p.merchant.take()) else {
            return false;
        };
        let uuid = host
            .hub
            .guests
            .iter()
            .find(|g| g.conn == conn)
            .map(|g| uuid_words(g.uuid));
        if std::mem::take(&mut gm.merchant.session.last_traded)
            && let (Some(id), Some(uuid)) = (gm.merchant.mob, uuid)
        {
            self.villager_traded_by(id, uuid);
        }
        self.merchant_closed(&mut gm.merchant);
        true
    }
    /// `Villager.customServerAiStep` for the guests' trading villagers: the
    /// TRADE reputation event, then the prices follow.
    pub(super) fn lan_merchant_tick(&mut self, host: &mut LanHost) {
        // `AbstractVillager.stillValid`: the trade screen closes once the trader
        // is gone or dead, or the guest is more than 4 blocks past its reach.
        let far: Vec<(u32, i32)> =
            host.play
                .iter()
                .filter_map(|(conn, p)| {
                    let gm = p.merchant.as_ref()?;
                    let id = gm.merchant.mob?;
                    let g = host
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.conn == *conn && host.hub.here(g))?;
                    let valid = self.mobs.iter().find(|m| m.id == id).is_some_and(|m| {
                        m.alive() && guest_within_entity_range(g, &m.bounds(), 4.0)
                    });
                    (!valid).then_some((*conn, gm.window))
                })
                .collect();
        for (conn, window) in far {
            if self.lan_merchant_closed(host, conn, Some(window)) {
                host.hub.send_to(
                    conn,
                    &C::ContainerClose(play::ContainerClose { container: window }),
                );
            }
        }
        let conns: Vec<u32> = host
            .play
            .iter()
            .filter(|(_, p)| {
                p.merchant
                    .as_ref()
                    .is_some_and(|m| m.merchant.session.villager && m.merchant.session.last_traded)
            })
            .map(|(c, _)| *c)
            .collect();
        for conn in conns {
            let Some(uuid) = host
                .hub
                .guests
                .iter()
                .find(|g| g.conn == conn && host.hub.here(g))
                .map(|g| uuid_words(g.uuid))
            else {
                continue;
            };
            let Some(gm) = host.play.get_mut(&conn).and_then(|p| p.merchant.as_mut()) else {
                continue;
            };
            gm.merchant.session.last_traded = false;
            let Some(id) = gm.merchant.mob else { continue };
            self.villager_traded_by(id, uuid);
            let reputation = self.villager_reputation_of(id, uuid);
            self.lan_merchant_op(host, conn, None, |menu, env| {
                if let Some(s) = menu.merchant.as_mut() {
                    s.reputation = reputation;
                    s.update_special_prices();
                }
                menu.merchant_update(env);
            });
        }
    }
    /// Takes a station a guest's action opened: a trading screen or a
    /// workstation menu goes to the guest (game/lan_stations.rs).
    pub(super) fn lan_station_opened(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        station: super::stations::Station,
    ) {
        let mut station = station;
        if station.kind == StationKind::Merchant
            && let Some(m) = station.merchant.take()
        {
            self.lan_open_merchant(host, conn, *m);
            return;
        }
        self.lan_open_station(host, conn, &station);
    }
}
