//! The merchant (villager trading) menu: `MerchantMenu` / `MerchantScreen`
//! layout (276x166, drawn 50 px left of the usual 176-wide origin), offer
//! buttons, payment slots, the result slot's `onTake`, villager experience and
//! level-ups with new offers from the next trade set.
use super::{
    Game,
    stations::{INPUT, Kind, RESULT, Station},
};
use horizon_client::mobs::{
    Species,
    render::{PROFESSIONS, VILLAGER_TYPES},
    villager,
};
use horizon_client::{
    hud::{HudBatch, rgba},
    items::ItemStack,
    merchant::{self, MerchantOffer, TradeContext},
};

/// The merchant screen is 100 px wider than the 176-px menus it shares an origin with.
const SHIFT: f32 = -50.0;
const GREY: u32 = 0xFF40_4040;

pub(super) struct MerchantState {
    /// The merchant's side of the menu: offers, experience, level, the
    /// selection hint and active offer (`horizon_client::merchant::Session`).
    pub(super) session: Box<merchant::Session>,
    /// The offer list's scroll offset.
    pub(super) scroll: usize,
    /// The offer button that keeps focus after a click (drawn highlighted).
    pub(super) focused: Option<usize>,
    /// The trading mob (its offers and experience are written back on
    /// close) and its custom name.
    pub(super) mob: Option<u32>,
    pub(super) name: Option<String>,
}
impl MerchantState {
    /// `Merchant.showProgressBar`: villagers only.
    pub(super) fn progress(&self) -> bool {
        self.session.show_progress()
    }
}
/// Player inventory slot positions in `MerchantMenu` (x from 108).
pub(super) fn inventory_slot(i: usize) -> (f32, f32) {
    match i {
        0..9 => (108.0 + SHIFT + i as f32 * 18.0, 142.0),
        _ => (
            108.0 + SHIFT + ((i - 9) % 9) as f32 * 18.0,
            84.0 + ((i - 9) / 9) as f32 * 18.0,
        ),
    }
}

/// `ExplorationMapFunction` in the game world: the nearest structure of the
/// tag through the generator's `/locate`, a new map centred on it with its
/// target decoration.
pub(super) struct GameMaps<'a> {
    pub(super) terrain: Option<std::sync::Arc<horizon_client::vanillagen::VanillaTerrain>>,
    /// Structure tag -> its structures.
    pub(super) structure_tag: &'a dyn Fn(&str) -> Vec<String>,
    /// Maps made, for the caller to register: (map id, centre x, z, scale, target).
    pub(super) made: std::cell::RefCell<Vec<(i32, [i32; 3], u8, String)>>,
    pub(super) next_id: std::cell::Cell<i32>,
}
impl merchant::MapMaker for GameMaps<'_> {
    fn exploration_map(
        &self,
        stack: ItemStack,
        map: &merchant::ExplorationMap,
        origin: [f64; 3],
    ) -> ItemStack {
        let Some(terrain) = self.terrain.as_ref() else {
            return stack;
        };
        let names = (self.structure_tag)(&map.destination);
        if names.is_empty() {
            return stack;
        }
        let from = [
            origin[0].floor() as i32,
            origin[1].floor() as i32,
            origin[2].floor() as i32,
        ];
        let Some((_, at)) = terrain.locate(&names, from, map.search_radius) else {
            return stack;
        };
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.made
            .borrow_mut()
            .push((id, at, map.zoom, map.decoration.clone()));
        // The trade's own map item (woodland_mansion_map, ...) keeps its id.
        let mut out = stack;
        let d = out.data_mut();
        d.map_id = Some(id);
        d.other.push((
            "minecraft:map_decorations".into(),
            horizon_world::nbt::Tag::compound([(
                "+",
                horizon_world::nbt::Tag::compound([
                    (
                        "type",
                        horizon_world::nbt::Tag::String(map.decoration.as_str().into()),
                    ),
                    (
                        "x",
                        horizon_world::nbt::Tag::Double(f64::from(at[0]).to_bits()),
                    ),
                    (
                        "z",
                        horizon_world::nbt::Tag::Double(f64::from(at[2]).to_bits()),
                    ),
                    (
                        "rotation",
                        horizon_world::nbt::Tag::Float(180.0f32.to_bits()),
                    ),
                ]),
            )]),
        ));
        out
    }
}

impl Game<'_> {
    pub(super) fn random_sequences_path(&self) -> std::path::PathBuf {
        self.world_root.join("data/minecraft/random_sequences.dat")
    }
    /// `RandomSequences` as `random_sequences.dat` (not on a server's guest).
    pub(super) fn save_random_sequences(&mut self) {
        if self.remote.is_some() || self.rand_sequences.ids().is_empty() {
            return;
        }
        let tag = horizon_world::nbt::Tag::compound([
            ("data", self.rand_sequences.to_nbt()),
            (
                "DataVersion",
                horizon_world::nbt::Tag::Int(horizon_world::DATA_VERSION),
            ),
        ]);
        if let Err(e) = horizon_client::save::write_dat(&self.random_sequences_path(), tag) {
            log::warn!("the villager random sequences were not written: {e}");
        }
    }
    /// `AbstractVillager.addOffersFromTradeSet` with the world's random
    /// sequence of the set, at the merchant's position.
    fn offers_from_set(
        &mut self,
        set: &str,
        variant: &str,
        origin: [f64; 3],
    ) -> Vec<MerchantOffer> {
        let Some(sequence) = self.res.stations().trades.sequence(set) else {
            return Vec::new();
        };
        let mut random = self.rand_sequences.lend(&sequence, self.level.seed);
        let next_map = self.peek_map_id();
        let tag = |t: &str| self.structure_tag(t);
        let maps = GameMaps {
            terrain: self.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let ctx = TradeContext {
            items: self.res.items(),
            enchantments: &self.res.stations().enchantments,
            variant,
            origin,
            maps: Some(&maps),
        };
        let offers = self
            .res
            .stations()
            .trades
            .offers_from_set(&ctx, set, &mut random);
        let made = maps.made.into_inner();
        self.rand_sequences.give_back(sequence, random);
        self.register_exploration_maps(made);
        offers
    }
    /// `AbstractVillager.getOffers`: generated on first use.
    pub(super) fn ensure_offers(&mut self, index: usize) {
        let species = self.mobs[index].species();
        let unset = self.mobs[index]
            .extra
            .trade
            .as_ref()
            .is_none_or(|t| t.unset);
        if unset {
            let m = &self.mobs[index];
            let variant = format!(
                "minecraft:{}",
                VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()]
            );
            let origin = [m.position.x, m.position.y, m.position.z];
            let offers = if species == Species::WanderingTrader {
                let mut v = Vec::new();
                for set in ["buying", "uncommon", "common"] {
                    v.extend(self.offers_from_set(
                        &format!("minecraft:wandering_trader/{set}"),
                        &variant,
                        origin,
                    ));
                }
                v
            } else {
                let profession = PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()];
                let level = i32::from(m.extra.variant3.max(1));
                let set = merchant::Trades::set_for(profession, level);
                if self.res.stations().trades.sets.contains_key(&set) {
                    self.offers_from_set(&set, &variant, origin)
                } else {
                    Vec::new()
                }
            };
            let trade = self.mobs[index].extra.trade.get_or_insert_default();
            trade.offers = offers;
            trade.unset = false;
        }
    }
    /// `Villager.mobInteract` / `WanderingTrader.mobInteract`: shake the
    /// head without offers, else open the trading screen.
    pub(super) fn interact_trader(
        &mut self,
        index: usize,
        item: &str,
    ) -> horizon_client::Result<bool> {
        let m = &self.mobs[index];
        let species = m.species();
        if !matches!(species, Species::Villager | Species::WanderingTrader)
            || !m.alive()
            || item == "minecraft:villager_spawn_egg"
            || self.station.is_some()
            || m.extra.flag2
            || (species == Species::Villager && villager::sleeping(m))
        {
            return Ok(false);
        }
        if m.is_baby() {
            // A baby villager shakes its head; a baby wandering trader does
            // nothing of its own.
            if species == Species::WanderingTrader {
                return Ok(false);
            }
            let mut events = Vec::new();
            villager::set_unhappy(&mut self.mobs[index], &mut events);
            self.handle_mob_events(events)?;
            return Ok(true);
        }
        self.stats.custom("talked_to_villager", 1);
        self.ensure_offers(index);
        let m = &self.mobs[index];
        let trade = m.extra.trade.as_deref().cloned().unwrap_or_default();
        if trade.offers.is_empty() {
            if species == Species::Villager {
                let mut events = Vec::new();
                villager::set_unhappy(&mut self.mobs[index], &mut events);
                self.handle_mob_events(events)?;
            }
            return Ok(true);
        }
        let villager = species == Species::Villager;
        let profession = if villager {
            PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()]
        } else {
            "wandering_trader"
        };
        let level = if villager {
            i32::from(m.extra.variant3.max(1)).min(5)
        } else {
            1
        };
        let mut session = merchant::Session::new(trade.offers, villager);
        session.xp = trade.xp;
        session.level = level;
        session.profession = format!("minecraft:{profession}");
        session.variant = format!(
            "minecraft:{}",
            VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()]
        );
        session.random = m.random.clone();
        session.origin = [m.position.x, m.position.y, m.position.z];
        session.ambient_sound_time = m.ambient_timer;
        let (id, name) = (m.id, m.custom_name.clone());
        if villager {
            session.reputation = self.villager_reputation(id);
            session.hero = self.hero_amplifier();
            for seq in merchant::Session::sequences_needed(
                &self.res.stations().trades,
                &session.profession,
                level,
            ) {
                let r = self.rand_sequences.lend(&seq, self.level.seed);
                session.sequences.insert(seq, r);
            }
        }
        // startTrading: updateSpecialPrices, then the screen opens.
        session.update_special_prices();
        // setTradingPlayer: a wandering trader stands still while trading, a
        // villager's brain follows and looks at the player
        // (LookAndFollowTradingPlayerSink).
        self.mobs[index].extra.flag2 = true;
        self.open_merchant_state(MerchantState {
            session: Box::new(session),
            scroll: 0,
            focused: None,
            mob: Some(id),
            name,
        });
        Ok(true)
    }
    /// `Villager.setTradingPlayer(null)`: the session goes back to the mob
    /// (special prices stay as they were), the borrowed sequences to the world.
    pub(super) fn merchant_closed(&mut self, state: &mut MerchantState) {
        let session = &mut state.session;
        for (id, r) in std::mem::take(&mut session.sequences) {
            self.rand_sequences.give_back(id, r);
        }
        let traded = std::mem::take(&mut session.last_traded);
        let Some(id) = state.mob else { return };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if session.villager {
            m.extra.variant3 = session.level.clamp(1, 5) as u8;
        }
        m.extra.flag2 = false;
        m.random = session.random.clone();
        m.ambient_timer = session.ambient_sound_time;
        let trade = m.extra.trade.get_or_insert_default();
        trade.xp = session.xp;
        trade.offers = session.offers.clone();
        if traded && session.villager {
            self.villager_traded(id);
        }
    }
    /// Opens a merchant menu without a mob (horizon-ctl `trade`).
    pub(super) fn open_merchant(&mut self, profession: &str, variant: &str, level: i32) {
        let profession = format!("minecraft:{}", profession.trim_start_matches("minecraft:"));
        let variant = format!("minecraft:{}", variant.trim_start_matches("minecraft:"));
        let level = level.clamp(1, 5);
        let origin = self
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let mut offers = Vec::new();
        for l in 1..=level {
            let set = merchant::Trades::set_for(&profession, l);
            offers.extend(self.offers_from_set(&set, &variant, origin));
        }
        let mut session = merchant::Session::new(offers, true);
        session.profession = profession;
        session.variant = variant;
        session.level = level;
        session.xp = merchant::min_xp(level);
        session.origin = origin;
        self.open_merchant_state(MerchantState {
            session: Box::new(session),
            scroll: 0,
            focused: None,
            mob: None,
            name: None,
        });
    }
    /// A server's trading window (no mob here: the server keeps it).
    pub(super) fn open_remote_merchant(
        &mut self,
        session: merchant::Session,
        name: Option<String>,
    ) {
        self.open_merchant_state(MerchantState {
            session: Box::new(session),
            scroll: 0,
            focused: None,
            mob: None,
            name,
        });
    }
    fn open_merchant_state(&mut self, state: MerchantState) {
        let mut s = Station::new_kind(Kind::Merchant, [0, 0, 0]);
        s.merchant = Some(Box::new(state));
        self.station = Some(s);
        self.screen = super::Screen::Inventory;
        self.capture(false);
        self.right = false;
    }
    /// The trading mob's random before a menu click (it ticks meanwhile).
    pub(super) fn merchant_sync_in(&self, session: &mut merchant::Session, mob: Option<u32>) {
        if let Some(m) = mob.and_then(|id| self.mobs.iter().find(|m| m.id == id)) {
            session.random = m.random.clone();
            session.ambient_sound_time = m.ambient_timer;
            session.origin = [m.position.x, m.position.y, m.position.z];
        }
    }
    /// After a menu click: the session back to the station, its random to
    /// the mob, and what it did (orbs, level-ups, the yes/no voice).
    pub(super) fn merchant_sync_out(&mut self, session: Box<merchant::Session>) {
        let mob = self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .and_then(|m| m.mob);
        let (session, traded) = self.merchant_apply(session, mob);
        self.merchant_traded(mob, &traded);
        if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
            m.session = session;
        }
    }
    /// What a menu click's session did, applied to the world and its mob;
    /// with the result of each trade made, for `merchant_traded`.
    pub(super) fn merchant_apply(
        &mut self,
        session: Box<merchant::Session>,
        mob: Option<u32>,
    ) -> (Box<merchant::Session>, Vec<ItemStack>) {
        let mut session = session;
        let mut traded = Vec::new();
        let events = std::mem::take(&mut session.events);
        let (villager, origin) = (session.villager, session.origin);
        if let Some(m) = mob.and_then(|id| self.mobs.iter_mut().find(|m| m.id == id)) {
            m.random = session.random.clone();
            m.ambient_timer = session.ambient_sound_time;
            if villager {
                m.extra.variant3 = session.level.clamp(1, 5) as u8;
            }
        }
        for e in events {
            match e {
                merchant::Event::Orb(v) => {
                    self.add_orb([origin[0], origin[1] + 0.5, origin[2]], v as u32)
                }
                merchant::Event::LevelUp(_) => {
                    if let Some(m) = mob.and_then(|id| self.mobs.iter_mut().find(|m| m.id == id)) {
                        m.add_effect("minecraft:regeneration", 200, 0);
                    }
                }
                merchant::Event::TradeUpdated(yes, pitch) => {
                    let sound = match (villager, yes) {
                        (true, true) => "minecraft:entity.villager.yes",
                        (true, false) => "minecraft:entity.villager.no",
                        (false, true) => "minecraft:entity.wandering_trader.yes",
                        (false, false) => "minecraft:entity.wandering_trader.no",
                    };
                    self.audio.play(sound, Some(origin), 1.0, pitch);
                }
                merchant::Event::Traded(index) => {
                    traded.extend(session.offers.get(index).map(|o| o.result.clone()));
                }
                merchant::Event::TradeSound => {}
            }
        }
        (session, traded)
    }
    /// `MerchantResultSlot.checkTakeAchievements` and `AbstractVillager.notifyTrade`
    /// for each trade made: the statistic and the `villager_trade` trigger.
    pub(super) fn merchant_traded(&mut self, mob: Option<u32>, results: &[ItemStack]) {
        for result in results {
            self.stats.custom("traded_with_villager", 1);
            let villager = mob
                .and_then(|id| self.mobs.iter().find(|m| m.id == id))
                .map(|m| self.mob_entity(m));
            if let Some(villager) = villager {
                self.trigger_trade(&villager, result);
            }
        }
    }
    /// `Villager.customServerAiStep`: the TRADE reputation event of the last
    /// trade, a tick later; the prices follow the new reputation.
    pub(super) fn merchant_tick(&mut self) {
        let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) else {
            return;
        };
        if !m.session.villager || !m.session.last_traded {
            return;
        }
        m.session.last_traded = false;
        let Some(id) = m.mob else { return };
        self.villager_traded(id);
        let reputation = self.villager_reputation(id);
        self.menu_session(|menu, env, _| {
            let s = menu.merchant.as_mut()?;
            s.reputation = reputation;
            s.update_special_prices();
            menu.merchant_update(env);
            Some(true)
        });
    }
    /// `MerchantMenu.tryMoveItems` for the selected offer (the select trade packet).
    fn try_move_items(&mut self, index: usize) {
        if self.remote.is_some() {
            // ServerboundSelectTradePacket; the server moves the items.
            if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
                m.session.selection_hint = index;
            }
            self.send(horizon_protocol::packets::play::Serverbound::SelectTrade(
                horizon_protocol::packets::play::SelectTrade { item: index as i32 },
            ));
            return;
        }
        self.menu_session(|menu, env, _| {
            menu.merchant_select(env, index);
            Some(true)
        });
    }
    /// Clicks on the offer list; true when one was hit.
    pub(super) fn merchant_click(&mut self, local: (f32, f32)) -> bool {
        let Some(m) = self.station.as_ref().and_then(|s| s.merchant.as_ref()) else {
            return false;
        };
        for i in 0..7 {
            let (x, y) = (5.0 + SHIFT, 18.0 + 20.0 * i as f32);
            if local.0 >= x
                && local.0 < x + 88.0
                && local.1 >= y
                && local.1 < y + 20.0
                && i + m.scroll < m.session.offers.len()
            {
                let index = i + m.scroll;
                if let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) {
                    m.focused = Some(i);
                }
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                self.try_move_items(index);
                return true;
            }
        }
        false
    }
    /// The offer buttons a controller can focus (GUI origin relative).
    pub(super) fn merchant_targets(&self) -> Vec<[f32; 4]> {
        let Some(m) = self.station.as_ref().and_then(|s| s.merchant.as_ref()) else {
            return Vec::new();
        };
        (0..7)
            .filter(|i| i + m.scroll < m.session.offers.len())
            .map(|i| [5.0 + SHIFT, 18.0 + 20.0 * i as f32, 88.0, 20.0])
            .collect()
    }
    pub(super) fn merchant_scroll(&mut self, d: f32) {
        let Some(m) = self.station.as_mut().and_then(|s| s.merchant.as_mut()) else {
            return;
        };
        if m.session.offers.len() > 7 {
            let max = m.session.offers.len() - 7;
            m.scroll = (m.scroll as i32 - d.signum() as i32).clamp(0, max as i32) as usize;
        }
    }
    /// `MerchantScreen` rendering.
    pub(super) fn draw_merchant(&mut self, batch: &mut HudBatch) -> bool {
        let (ox, oy) = self.inventory_origin(batch);
        let local = (self.cursor.0 - ox, self.cursor.1 - oy);
        let Some(m) = self.station.as_ref().and_then(|s| s.merchant.as_ref()) else {
            return false;
        };
        let (progress, custom) = (m.progress(), m.name.clone());
        let (profession, level, xp, scroll, shop, future, focused) = (
            m.session
                .profession
                .trim_start_matches("minecraft:")
                .to_owned(),
            m.session.level,
            m.session.xp,
            m.scroll,
            m.session.selection_hint,
            m.session.future_xp,
            m.focused,
        );
        let offers = m.session.offers.clone();
        let costs: Vec<ItemStack> = offers.iter().map(|o| o.cost_a(self.res.items())).collect();
        let atlas = &self.gui.atlas;
        batch.fill(
            atlas,
            0.0,
            0.0,
            batch.width,
            batch.height,
            rgba(16, 16, 16, 160),
        );
        if let Some(mut r) = atlas.sprites.get("gui/container/villager").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u1 = r.u0 + 276.0 * du;
            r.v1 = r.v0 + 166.0 * dv;
            batch.rect_uv(ox + SHIFT, oy, 276.0, 166.0, r, u32::MAX);
        }
        let e = &self.res.stations().enchantments;
        let tr = |k: &str, f: &str| e.translate(k).unwrap_or(f).to_owned();
        let name = custom.unwrap_or_else(|| {
            if progress {
                tr(
                    &format!("entity.minecraft.villager.{profession}"),
                    &profession,
                )
            } else {
                tr("entity.minecraft.wandering_trader", "Wandering Trader")
            }
        });
        let title = if progress {
            tr("merchant.title", "%s - %s")
                .replacen("%s", &name, 1)
                .replacen("%s", &tr(&format!("merchant.level.{level}"), ""), 1)
        } else {
            name
        };
        let tw = HudBatch::text_width(atlas, &title);
        batch.text(
            atlas,
            &title,
            ox + SHIFT + (49.0 + 138.0 - (tw / 2.0).floor()),
            oy + 6.0,
            GREY,
            false,
        );
        let trades = tr("merchant.trades", "Trades");
        let tw = HudBatch::text_width(atlas, &trades);
        batch.text(
            atlas,
            &trades,
            ox + SHIFT + 5.0 - (tw / 2.0).floor() + 48.0,
            oy + 6.0,
            GREY,
            false,
        );
        batch.text(
            atlas,
            &tr("container.inventory", "Inventory"),
            ox + SHIFT + 107.0,
            oy + 72.0,
            GREY,
            false,
        );
        if offers.get(shop).is_some_and(MerchantOffer::out_of_stock) {
            batch.sprite(
                atlas,
                "gui/sprites/container/villager/out_of_stock",
                ox + SHIFT + 182.0,
                oy + 35.0,
            );
        }
        // Experience bar (levels 1-4).
        if progress && level < 5 {
            batch.sprite(
                atlas,
                "gui/sprites/container/villager/experience_bar_background",
                ox + SHIFT + 136.0,
                oy + 16.0,
            );
            let min = merchant::min_xp(level);
            if xp >= min && merchant::can_level_up(level) {
                let mult = 102.0 / (merchant::max_xp(level) - min) as f32;
                let w = ((mult * (xp - min) as f32).floor()).min(102.0);
                batch.sprite_part(
                    atlas,
                    "gui/sprites/container/villager/experience_bar_current",
                    ox + SHIFT + 136.0,
                    oy + 16.0,
                    w / 102.0,
                );
                if future > 0 {
                    let fw = (future as f32 * mult).floor().min(102.0 - w);
                    if let Some(mut r) = atlas
                        .sprites
                        .get("gui/sprites/container/villager/experience_bar_result")
                        .copied()
                    {
                        let du = (r.u1 - r.u0) / r.w as f32;
                        r.u0 += w * du;
                        r.u1 = r.u0 + fw * du;
                        batch.rect_uv(ox + SHIFT + 136.0 + w, oy + 16.0, fw, 5.0, r, u32::MAX);
                    }
                }
            }
        }
        // Scroller.
        let steps = offers.len() as i32 + 1 - 7;
        if steps > 1 {
            let left = 139 - (27 + (steps - 1) * 139 / steps);
            let step = 1 + left / steps + 139 / steps;
            let mut y = (scroll as i32 * step).min(113);
            if scroll as i32 == steps - 1 {
                y = 113;
            }
            batch.sprite(
                atlas,
                "gui/sprites/container/villager/scroller",
                ox + SHIFT + 94.0,
                oy + 18.0 + y as f32,
            );
        } else {
            batch.sprite(
                atlas,
                "gui/sprites/container/villager/scroller_disabled",
                ox + SHIFT + 94.0,
                oy + 18.0,
            );
        }
        // Offer buttons.
        let mut tooltip: Option<ItemStack> = None;
        for (row, (index, offer)) in offers.iter().enumerate().skip(scroll).take(7).enumerate() {
            let (bx, by) = (ox + SHIFT + 5.0, oy + 18.0 + 20.0 * row as f32);
            let hover = local.0 >= 5.0 + SHIFT
                && local.0 < 93.0 + SHIFT
                && local.1 >= 18.0 + 20.0 * row as f32
                && local.1 < 38.0 + 20.0 * row as f32;
            let sprite = if hover || focused == Some(row) {
                "gui/sprites/widget/button_highlighted"
            } else {
                "gui/sprites/widget/button"
            };
            batch.sprite_scaled(&self.gui.atlas, sprite, bx, by, 88.0, 20.0, u32::MAX);
            let y = oy + 16.0 + 1.0 + 20.0 * row as f32 + 2.0;
            let base = offer.base_cost_a.stack.clone();
            let cost_a = costs[index].clone();
            let x = ox + SHIFT + 10.0;
            if base.count == cost_a.count {
                self.draw_item(batch, &cost_a, x, y);
            } else {
                // Discounted: the base price struck through, then the new price.
                self.draw_item(
                    batch,
                    &ItemStack {
                        count: 1,
                        ..cost_a.clone()
                    },
                    x,
                    y,
                );
                let atlas = &self.gui.atlas;
                let t = base.count.to_string();
                batch.text(
                    atlas,
                    &t,
                    x + 17.0 - HudBatch::text_width(atlas, &t),
                    y + 9.0,
                    u32::MAX,
                    true,
                );
                let t = cost_a.count.to_string();
                batch.text(
                    atlas,
                    &t,
                    x + 14.0 + 17.0 - HudBatch::text_width(atlas, &t),
                    y + 9.0,
                    u32::MAX,
                    true,
                );
                batch.sprite(
                    atlas,
                    "gui/sprites/container/villager/discount_strikethrough",
                    x + 7.0,
                    y + 12.0,
                );
            }
            if let Some(b) = offer.cost_b() {
                self.draw_item(batch, &b, ox + SHIFT + 40.0, y);
            }
            let arrow = if offer.out_of_stock() {
                "trade_arrow_out_of_stock"
            } else {
                "trade_arrow"
            };
            batch.sprite(
                &self.gui.atlas,
                &format!("gui/sprites/container/villager/{arrow}"),
                ox + SHIFT + 60.0,
                y + 3.0,
            );
            self.draw_item(batch, &offer.result, ox + SHIFT + 73.0, y);
            if hover && self.carried.is_none() {
                let rel = local.0 - (5.0 + SHIFT);
                tooltip = if rel < 20.0 {
                    Some(cost_a)
                } else if rel > 30.0 && rel < 50.0 {
                    offer.cost_b()
                } else if rel > 65.0 {
                    Some(offer.result.clone())
                } else {
                    None
                };
            }
        }
        // Slots, hover highlight, carried stack, tooltip.
        let slots: Vec<usize> = (0..36).chain([INPUT, INPUT + 1, RESULT]).collect();
        let drag = self.drag_view();
        for i in slots {
            let (sx, sy) = self.slot_position(i);
            let stack = self.slot_mut(i).clone();
            self.draw_slot_item(batch, drag.as_ref(), i, &stack, ox + sx, oy + sy);
        }
        if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
            let (sx, sy) = self.slot_position(slot);
            batch.fill(
                &self.gui.atlas,
                ox + sx,
                oy + sy,
                16.0,
                16.0,
                rgba(255, 255, 255, 128),
            );
            if self.carried.is_none() && tooltip.is_none() {
                tooltip = self.slot_mut(slot).clone();
            }
        }
        // An out-of-stock trade: "the villager needs to restock" over the
        // result (isHovering(186, 35, 22, 21), canRestock).
        let deprecated = progress
            && offers.get(shop).is_some_and(MerchantOffer::out_of_stock)
            && local.0 >= 186.0 + SHIFT - 1.0
            && local.0 < 186.0 + SHIFT + 22.0 + 1.0
            && local.1 >= 35.0 - 1.0
            && local.1 < 35.0 + 21.0 + 1.0;
        if self.carried.is_some() {
            self.draw_carried(batch);
        } else if deprecated {
            let text = self
                .res
                .stations()
                .enchantments
                .translate("merchant.deprecated")
                .unwrap_or("Villagers restock up to two times per day.")
                .to_owned();
            let lines: Vec<(String, u32)> =
                text.split('\n').map(|l| (l.to_owned(), u32::MAX)).collect();
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        } else if let Some(stack) = tooltip {
            self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
        }
        true
    }
}
