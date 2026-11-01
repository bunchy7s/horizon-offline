//! Hub operations on what guests perceive and do beyond the basics: sounds,
//! level events, animations and block containers with their clicks.
use super::{
    hub::Hub,
    menu::{self, Menu, NativeMenuEvent, Rules, Slots, View},
};
use crate::{containers::BlockEntity, items::ItemStack, world::World};
use horizon_core::random::RandomSource;
use horizon_protocol::{
    item::Hashed,
    packets::play::{self, Clientbound as C},
};
use horizon_world::nbt::Tag;
use std::collections::HashMap;

impl Hub {
    pub fn send_to(&self, conn: u32, packet: &C) {
        if let Some(g) = self.guest(conn) {
            g.send(packet);
        }
    }

    /// `ChunkMap.sendToTrackingPlayers`: send to every placed guest that
    /// tracks the entity `id`.
    pub fn send_to_tracking(&self, id: i32, packet: &C) {
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.here(g) && g.tracker.is_tracking(id))
        {
            g.sender.send(body.clone());
        }
    }

    /// Send to every placed guest within `range` blocks of `at`.
    pub fn broadcast_near(&self, at: [f64; 3], range: f64, packet: &C, except: Option<u32>) {
        let body = packet.encode();
        for g in self
            .guests
            .iter()
            .filter(|g| self.here(g) && Some(g.conn) != except)
        {
            let d: f64 = (0..3).map(|k| (g.position[k] - at[k]).powi(2)).sum();
            if d <= range * range {
                g.sender.send(body.clone());
            }
        }
    }

    /// ClientboundSoundPacket for a sound event name (an inline event when
    /// the registry does not know it).
    pub fn sound_packet(
        &self,
        name: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: i32,
        seed: i64,
    ) -> C {
        let sound = match self.sync.id("minecraft:sound_event", name) {
            Some(id) => (id, None),
            None => (-1, Some((name.to_owned(), None))),
        };
        C::Sound(play::Sound {
            sound,
            source,
            x: (at[0] * 8.0) as i32,
            y: (at[1] * 8.0) as i32,
            z: (at[2] * 8.0) as i32,
            volume,
            pitch,
            seed,
        })
    }
    /// Draw the next seed from the server sound source.
    pub fn next_sound_seed(&self) -> i64 {
        next_sound_seed(&mut *self.sound_seeds.lock().unwrap())
    }

    pub fn sound(
        &self,
        name: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        source: i32,
        except: Option<u32>,
    ) {
        // Level.playSound: soundSeedGenerator.nextLong per call, not the tick.
        let seed = self.next_sound_seed();
        let packet = self.sound_packet(name, at, volume, pitch, source, seed);
        self.broadcast_near(at, f64::from(volume.max(1.0)) * 16.0, &packet, except);
    }

    /// ClientboundLevelEventPacket (block break particles and sounds ...).
    pub fn level_event(&self, kind: i32, pos: [i32; 3], data: i32, except: Option<u32>) {
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        self.broadcast_near(
            at,
            64.0,
            &C::LevelEvent(play::LevelEvent {
                kind,
                pos,
                data,
                global: false,
            }),
            except,
        );
    }

    /// `ServerLevel.globalLevelEvent`: the packet with its global flag, for
    /// every player on the server (`PlayerList.broadcastAll`).
    pub fn global_level_event(&self, kind: i32, pos: [i32; 3], data: i32) {
        self.broadcast(&C::LevelEvent(play::LevelEvent {
            kind,
            pos,
            data,
            global: true,
        }));
    }

    /// ClientboundBlockEventPacket (`PlayerList.broadcast` within 64 blocks).
    pub fn block_event(&self, pos: [i32; 3], b0: i32, b1: i32, state: u32) {
        let block = self
            .encoder
            .block_ids
            .get(state as usize)
            .copied()
            .unwrap_or(0);
        let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
        self.broadcast_near(
            at,
            64.0,
            &C::BlockEvent(play::BlockEvent {
                pos,
                b0: b0 as u8,
                b1: b1 as u8,
                block,
            }),
            None,
        );
    }

    pub fn entity_event(&self, id: i32, event: i8, at: [f64; 3]) {
        self.broadcast_near(
            at,
            128.0,
            &C::EntityEvent(play::EntityEvent { id, event }),
            None,
        );
    }

    /// `ServerPlayer.crit` (`magicCrit` when `magic`): send the victim's
    /// animation to the players tracking the attacker and the attacker
    /// themselves (`sendToTrackingPlayersAndSelf`).
    pub fn critical_hit(&self, attacker: i32, target: i32, magic: bool) {
        let body = C::Animate(play::Animate {
            id: target,
            action: if magic { 2 } else { 1 },
        })
        .encode();
        for g in self.guests.iter().filter(|g| {
            self.here(g) && (g.entity_id == attacker || g.tracker.is_tracking(attacker))
        }) {
            g.sender.send(body.clone());
        }
    }

    /// `ServerLevel.broadcastDamageEvent`: the clients around show the hurt
    /// entity's flash and hear its hurt sound.
    pub fn damage_event(
        &self,
        id: i32,
        kind: &str,
        cause: Option<i32>,
        direct: Option<i32>,
        at: [f64; 3],
    ) {
        let Some(source_type) = self.sync.id("minecraft:damage_type", kind) else {
            return;
        };
        self.broadcast_near(
            at,
            128.0,
            &C::DamageEvent(play::DamageEvent {
                id,
                source_type,
                cause,
                direct,
                position: None,
            }),
            None,
        );
    }

    pub fn swing(&self, id: i32, hand: i32, at: [f64; 3], except: Option<u32>) {
        self.broadcast_near(
            at,
            128.0,
            &C::SwingAnimation(play::SwingAnimation {
                id,
                hand,
                kind: 1,
                duration: 6,
            }),
            except,
        );
    }

    pub fn take_item(&self, item: i32, collector: i32, count: i32, at: [f64; 3]) {
        self.broadcast_near(
            at,
            64.0,
            &C::TakeItemEntity(play::TakeItemEntity {
                item,
                player: collector,
                amount: count,
            }),
            None,
        );
    }

    /// Open a block container for a guest (ServerPlayer.openMenu).
    pub fn open_container(
        &mut self,
        conn: u32,
        menu_kind: &str,
        title: Tag,
        pos: (i32, i32, i32),
        slots: Slots,
        items: Vec<Option<ItemStack>>,
    ) {
        let Some(kind) = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id(menu_kind))
        else {
            return;
        };
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        self.close_container_at(i);
        let g = &mut self.guests[i];
        g.next_window = g.next_window % 100 + 1;
        let window = g.next_window;
        g.send(&C::OpenScreen(play::OpenScreen {
            container: window,
            menu: kind,
            title,
        }));
        let mod_type = crate::containers::mod_menu_type(menu_kind);
        g.menu = Some(Menu {
            window,
            kind,
            pos: Some(pos),
            slots,
            size: items.len(),
            state: 0,
            sent: Vec::new(),
            drag: None,
            sent_data: Vec::new(),
            hidden: Vec::new(),
            results: mod_type.map_or_else(Vec::new, |t| t.results.clone()),
            data_slots: mod_type.map(|t| t.data_slots),
        });
        self.send_menu_content(i, items);
    }

    /// The open menu of `conn` has no `hidden` container slots: a mod's screen
    /// does not show them, so a click or a shift-click never reaches them.
    pub fn hide_menu_slots(&mut self, conn: u32, hidden: Vec<usize>) {
        if let Some(m) = self
            .guests
            .iter_mut()
            .find(|g| g.conn == conn)
            .and_then(|g| g.menu.as_mut())
        {
            m.hidden = hidden;
        }
    }

    /// Opens a native mod's menu of `rows` rows (1 to 6, a chest's
    /// `generic_9xN`) showing `items` (padded to the rows' slots), over the
    /// guest's inventory: no click moves anything, each is a
    /// [`NativeMenuEvent`]. The window's id, or None for an unknown guest.
    pub fn open_native_menu(
        &mut self,
        conn: u32,
        rows: u8,
        title: Tag,
        mut items: Vec<Option<ItemStack>>,
    ) -> Option<i32> {
        let rows = rows.clamp(1, 6);
        let kind = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id(&format!("minecraft:generic_9x{rows}")))?;
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        let size = usize::from(rows) * 9;
        items.resize(size, None);
        let g = &mut self.guests[i];
        g.native_menu = items.clone();
        g.next_window = g.next_window % 100 + 1;
        let window = g.next_window;
        g.send(&C::OpenScreen(play::OpenScreen {
            container: window,
            menu: kind,
            title,
        }));
        g.menu = Some(Menu {
            window,
            kind,
            pos: None,
            slots: Slots::Native,
            size,
            state: 0,
            sent: Vec::new(),
            drag: None,
            sent_data: Vec::new(),
            hidden: Vec::new(),
            results: Vec::new(),
            data_slots: None,
        });
        self.send_menu_content(i, items);
        Some(window)
    }

    /// Replaces what a guest's open native menu shows; the changed slots go
    /// out with the tick's container sync.
    pub fn set_native_menu(&mut self, conn: u32, mut items: Vec<Option<ItemStack>>) {
        if let Some(g) = self.guest_mut(conn)
            && let Some(m) = g.menu.as_ref().filter(|m| m.slots == Slots::Native)
        {
            items.resize(m.size, None);
            g.native_menu = items;
        }
    }

    /// Open a trading window for a guest (`Merchant.openTradingScreen`):
    /// its id, or None when the menu type is unknown.
    pub fn open_merchant_window(&mut self, conn: u32, title: Tag) -> Option<i32> {
        self.open_window(conn, "minecraft:merchant", title)
    }

    /// ServerPlayer.openMenu for a menu the game runs (merchant, crafting
    /// table): the open container closes, a new window id opens this menu.
    pub fn open_window(&mut self, conn: u32, menu: &str, title: Tag) -> Option<i32> {
        let kind = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id(menu))?;
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        let g = &mut self.guests[i];
        g.next_window = g.next_window % 100 + 1;
        let window = g.next_window;
        g.send(&C::OpenScreen(play::OpenScreen {
            container: window,
            menu: kind,
            title,
        }));
        Some(window)
    }

    /// ServerPlayer.openHorseInventory: the open container closes, a new
    /// window id opens mount `entity`'s inventory with its chest's columns
    /// (ClientboundMountScreenOpenPacket).
    pub fn open_mount_window(&mut self, conn: u32, columns: i32, entity: i32) -> Option<i32> {
        let i = self.guests.iter().position(|g| g.conn == conn)?;
        self.close_container_at(i);
        let g = &mut self.guests[i];
        g.next_window = g.next_window % 100 + 1;
        let window = g.next_window;
        g.send(&C::MountScreenOpen(play::MountScreenOpen {
            container: window,
            columns,
            entity,
        }));
        Some(window)
    }

    fn send_menu_content(&mut self, i: usize, mut items: Vec<Option<ItemStack>>) {
        for j in 0..36 {
            items.push(self.guests[i].inventory[player_slot(j)].clone());
        }
        let net: Vec<_> = items.iter().map(|s| self.stack(s)).collect();
        let carried = self.guests[i].carried.clone();
        let carried = self.stack(&carried);
        let g = &mut self.guests[i];
        let Some(menu) = g.menu.as_mut() else {
            return;
        };
        menu.state = (menu.state + 1) & 0x7FFF;
        let packet = C::ContainerSetContent(play::ContainerSetContent {
            container: menu.window,
            state: menu.state,
            items: net,
            carried,
        });
        menu.sent = items;
        g.send(&packet);
        g.inventory_told();
    }

    /// `AbstractContainerMenu.sendAllDataToRemote` for the guest's open
    /// block container `window`: every slot and the carried stack again.
    pub fn resend_menu(&mut self, conn: u32, window: i32, world: &World) {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let g = &self.guests[i];
        let Some(items) = g
            .menu
            .as_ref()
            .filter(|m| window != 0 && m.window == window)
            .and_then(|m| m.slots.read(world, g))
        else {
            return;
        };
        self.send_menu_content(i, items);
    }

    fn close_container_at(&mut self, i: usize) {
        let g = &mut self.guests[i];
        if let Some(menu) = g.menu.take() {
            g.send(&C::ContainerClose(play::ContainerClose {
                container: menu.window,
            }));
            if menu.slots == Slots::Native {
                g.native_menu.clear();
                let (uuid, window) = (g.uuid, menu.window);
                self.native_menu_events
                    .push(NativeMenuEvent::Closed { uuid, window });
            }
        }
    }

    pub(super) fn close_container_for(&mut self, i: usize) {
        self.close_container_at(i);
    }

    pub fn close_container(&mut self, conn: u32) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
            self.close_container_at(i);
        }
    }

    /// The client closed its window.
    pub fn container_closed(&mut self, conn: u32, window: i32) {
        if let Some(g) = self.guest_mut(conn)
            && g.menu.as_ref().is_some_and(|m| m.window == window)
        {
            let native = g.menu.take().is_some_and(|m| m.slots == Slots::Native);
            if native {
                g.native_menu.clear();
                let uuid = g.uuid;
                self.native_menu_events
                    .push(NativeMenuEvent::Closed { uuid, window });
            }
        }
    }

    fn hashed_matches(&mut self, claimed: &Option<Hashed>, mine: &Option<ItemStack>) -> bool {
        match (claimed, mine) {
            (None, None) => true,
            (Some(h), Some(s)) => h.item == self.item_id(&s.item) && h.count == s.count as i32,
            _ => false,
        }
    }

    fn view_slot(&self, i: usize, world: &World, slot: usize) -> Option<ItemStack> {
        let g = &self.guests[i];
        match &g.menu {
            Some(m) if slot < m.size => m.slots.get(world, g, slot),
            Some(m) => g
                .inventory
                .get(player_slot(slot - m.size))
                .cloned()
                .flatten(),
            None => g.menu_slot(slot),
        }
    }

    /// ServerGamePacketListenerImpl.handleContainerClick. Returns the stacks
    /// thrown out of the menu.
    pub fn container_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        world: &mut World,
        rules: &dyn Rules,
    ) -> Vec<ItemStack> {
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return Vec::new();
        };
        let in_menu = p.container != 0;
        let window_ok = !in_menu
            || self.guests[i]
                .menu
                .as_ref()
                .is_some_and(|m| m.window == p.container);
        if !window_ok {
            return Vec::new();
        }
        if self.guests[i].spectator() {
            // A spectator's click changes nothing, and sendAllDataToRemote
            // takes back what its client predicted.
            self.resend_menu(conn, p.container, world);
            return Vec::new();
        }
        // A native mod's menu: the click is the mod's to answer, and the
        // whole menu and the cursor go back as they were.
        if in_menu && self.guests[i].menu.as_ref().map(|m| m.slots) == Some(Slots::Native) {
            self.native_menu_events.push(NativeMenuEvent::Click {
                uuid: self.guests[i].uuid,
                window: p.container,
                slot: i32::from(p.slot),
                button: i32::from(p.button),
                mode: p.input,
            });
            self.resend_menu(conn, p.container, world);
            return Vec::new();
        }
        // A menu that is no longer valid takes no clicks.
        if let Some(m) = self.guests[i].menu.as_ref().filter(|_| in_menu)
            && !m.slots.still_valid(world, &self.guests[i], m.pos)
        {
            return Vec::new();
        }
        // AbstractContainerMenu.isValidSlotIndex: -999 or below the menu's slot
        // count. A slot past the menu would reach the inventory beyond its 36 slots.
        let slots = match self.guests[i].menu.as_ref().filter(|_| in_menu) {
            Some(m) => m.size + 36,
            None => 46,
        };
        if i32::from(p.slot) >= slots as i32 && p.slot != -999 {
            return Vec::new();
        }
        let source = self.guests[i].menu.as_ref().map(|m| m.slots);
        let shulker_box = self
            .sync
            .builtin
            .get("minecraft:menu")
            .and_then(|m| m.id("minecraft:shulker_box"));
        let shulker = in_menu
            && shulker_box.is_some()
            && self.guests[i].menu.as_ref().map(|m| m.kind) == shulker_box;
        let mut drag = if in_menu {
            self.guests[i].menu.as_mut().and_then(|m| m.drag.take())
        } else {
            self.guests[i].drag.take()
        };
        let hidden = self.guests[i]
            .menu
            .as_ref()
            .filter(|_| in_menu)
            .map_or_else(Vec::new, |m| m.hidden.clone());
        let results = self.guests[i]
            .menu
            .as_ref()
            .filter(|_| in_menu)
            .map_or_else(Vec::new, |m| m.results.clone());
        let guest = &mut self.guests[i];
        let mut items = source.and_then(|s| s.read(world, guest));
        if in_menu && items.is_none() {
            return Vec::new();
        }
        let mut view = View {
            guest,
            container: items.as_deref_mut(),
            shulker,
            hidden: &hidden,
            results: &results,
        };
        let (drops, changed) = menu::click(
            &mut view,
            &mut drag,
            i32::from(p.slot),
            i32::from(p.button),
            p.input,
            rules,
        );
        if let (Some(source), Some(items)) = (source, items) {
            source.write(world, guest, items);
        }
        if in_menu {
            if let Some(m) = self.guests[i].menu.as_mut() {
                m.drag = drag;
            }
        } else {
            self.guests[i].drag = drag;
        }
        self.answer_click(i, p, world, &changed);
        drops
    }

    /// `AbstractContainerMenu.setSelectedBundleItemIndex` on the guest's open
    /// menu: a container's own slots, or the inventory's. A bundle's selection
    /// lives on the server's stack until the click that takes from it.
    pub fn select_bundle_item(&mut self, conn: u32, slot: usize, index: i32, world: &mut World) {
        let Some(g) = self.guests.iter_mut().find(|g| g.conn == conn) else {
            return;
        };
        let open = g.menu.as_ref().map(|m| (m.size, m.slots));
        let mut items = match open {
            Some((size, source)) if slot < size => source.read(world, g),
            _ => None,
        };
        let stack = match open {
            Some((size, _)) if slot < size => items
                .as_mut()
                .and_then(|items| items.get_mut(slot))
                .and_then(Option::as_mut),
            // setSelectedBundleItemIndex: a slot of the menu only.
            Some((size, _)) if slot - size < 36 => g
                .inventory
                .get_mut(player_slot(slot - size))
                .and_then(Option::as_mut),
            Some(_) => None,
            None => match slot {
                9..=35 => g.inventory.get_mut(slot).and_then(Option::as_mut),
                36..=44 => g.inventory.get_mut(slot - 36).and_then(Option::as_mut),
                45 => g.offhand.as_mut(),
                _ => None,
            },
        };
        if let Some(stack) = stack {
            crate::bundle::toggle_selected(stack, index);
        }
        if let (Some((_, source)), Some(items)) = (open, items) {
            source.write(world, g, items);
        }
    }

    /// A click on the inventory menu (window 0) the game applied itself
    /// (the crafting grid): the client is corrected like after any click.
    pub fn inventory_clicked(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        world: &World,
        changed: &[usize],
    ) {
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
            self.answer_click(i, p, world, changed);
        }
    }

    /// What the client predicted for click `p` is what the server has, for a
    /// menu of `total` slots read through `view`.
    pub fn click_agreed(
        &mut self,
        i: usize,
        p: &play::ContainerClick,
        total: usize,
        view: &dyn Fn(usize) -> Option<ItemStack>,
        changed: &[usize],
    ) -> bool {
        let carried = self.guests[i].carried.clone();
        if !self.hashed_matches(&p.carried, &carried) {
            return false;
        }
        let claimed: HashMap<usize, &Option<Hashed>> =
            p.changed.iter().map(|(s, h)| (*s as usize, h)).collect();
        (0..total).all(|slot| match claimed.get(&slot) {
            Some(c) => self.hashed_matches(c, &view(slot)),
            None => !changed.contains(&slot),
        })
    }

    /// After a click: tell the client about every slot it got wrong, or that
    /// it did not claim, and the carried stack.
    fn answer_click(
        &mut self,
        i: usize,
        p: &play::ContainerClick,
        world: &World,
        changed: &[usize],
    ) {
        let in_menu = p.container != 0;
        // handleContainerClick: a click made without the latest state gets the
        // whole menu (`broadcastFullState`). Answering it slot by slot can leave
        // the client with a correction of a click it has already gone past.
        let g = &self.guests[i];
        let open = g
            .menu
            .as_ref()
            .filter(|_| in_menu)
            .map(|m| (m.size, m.state));
        let stale = match open {
            Some((_, state)) => p.state != state,
            None => !in_menu && p.state != g.menu_state,
        };
        if stale {
            match open {
                Some((size, _)) => {
                    let items = (0..size).map(|s| self.view_slot(i, world, s)).collect();
                    self.send_menu_content(i, items);
                }
                None => self.send_inventory(i),
            }
            return;
        }
        let claimed: HashMap<usize, Option<Hashed>> = p
            .changed
            .iter()
            .map(|(s, h)| (*s as usize, h.clone()))
            .collect();
        let total = if in_menu {
            self.guests[i].menu.as_ref().map_or(0, |m| m.size) + 36
        } else {
            46
        };
        let mut resend: Vec<usize> = Vec::new();
        for slot in 0..total {
            let mine = self.view_slot(i, world, slot);
            let agreed = match claimed.get(&slot) {
                Some(c) => self.hashed_matches(c, &mine),
                None => !changed.contains(&slot),
            };
            if !agreed {
                resend.push(slot);
            }
        }
        for slot in resend {
            let mine = self.view_slot(i, world, slot);
            let net = self.stack(&mine);
            let g = &mut self.guests[i];
            let state = if in_menu {
                g.menu.as_mut().map_or(0, |m| {
                    m.state = (m.state + 1) & 0x7FFF;
                    m.state
                })
            } else {
                g.menu_state = (g.menu_state + 1) & 0x7FFF;
                g.menu_state
            };
            g.send(&C::ContainerSetSlot(play::ContainerSetSlot {
                container: p.container,
                state,
                slot: slot as i16,
                item: net,
            }));
        }
        let carried = self.guests[i].carried.clone();
        if !self.hashed_matches(&p.carried, &carried) {
            let net = self.stack(&carried);
            self.guests[i].send(&C::SetCursorItem(play::SetCursorItem { item: net }));
        }
        // What the client now believes equals the server's state.
        let items: Vec<_> = (0..total).map(|s| self.view_slot(i, world, s)).collect();
        if in_menu {
            // Nor is the inventory menu's sync to send what this click settled:
            // ServerPlayer.tick broadcasts only the open menu.
            let own = open.map_or(0, |(size, _)| size);
            let settled = claimed.keys().copied().chain(changed.iter().copied());
            for slot in settled.filter(|&s| s >= own && s < own + 36) {
                self.guests[i].menu_sent[slot - own + 9] = items[slot].clone();
            }
            if let Some(m) = self.guests[i].menu.as_mut() {
                m.sent = items;
            }
        } else {
            self.guests[i].menu_sent = items;
        }
    }

    /// AbstractContainerMenu.broadcastChanges' data slots: a furnace's
    /// AbstractFurnaceBlockEntity.dataAccess (lit time left, lit total, cooking
    /// timer, cooking total), each sent when it changes.
    fn sync_container_data(&mut self, i: usize, world: &World) {
        let g = &mut self.guests[i];
        let Some(m) = g.menu.as_mut() else {
            return;
        };
        let Slots::Block(pos) = m.slots else {
            return;
        };
        // AbstractFurnaceBlockEntity.dataAccess, BrewingStandBlockEntity.dataAccess,
        // CrafterBlockEntity.containerData.
        let data = match world.block_entities.get(&pos) {
            Some(BlockEntity::Furnace(f)) => vec![
                f.lit_remaining,
                f.lit_total,
                f.cooking_timer,
                f.cooking_total,
            ],
            Some(BlockEntity::Brewing(b)) => {
                vec![b.brew_time, b.fuel, b.total_brew_time, b.total_fuel]
            }
            // CrafterBlockEntity.containerData: the disabled slots, then triggered.
            Some(BlockEntity::Crafter(c)) => c
                .disabled
                .iter()
                .chain([&c.triggered])
                .map(|&d| i32::from(d))
                .collect(),
            // A mod's menu: the data slots its tick set. A mod menu type's
            // `ContainerData` has as many as it declares, all sent on open
            // (`sendInitialData`), 0 where the tick set none.
            Some(BlockEntity::Mod(e)) if m.data_slots.is_some() || !e.data.is_empty() => {
                let mut data = e.data.clone();
                if let Some(n) = m.data_slots {
                    data.resize(n, 0);
                }
                data
            }
            _ => return,
        }
        .into_iter()
        .map(|v| v as i16)
        .collect::<Vec<i16>>();
        m.sent_data.resize(data.len(), None);
        let window = m.window;
        let changed: Vec<(usize, i16)> = data
            .iter()
            .enumerate()
            .filter(|&(k, v)| m.sent_data[k] != Some(*v))
            .map(|(k, &v)| (k, v))
            .collect();
        for &(k, value) in &changed {
            m.sent_data[k] = Some(value);
        }
        for (k, value) in changed {
            g.send(&C::ContainerSetData(play::ContainerSetData {
                container: window,
                id: k as i16,
                value,
            }));
        }
    }

    /// Slot changes of open containers (AbstractContainerMenu.broadcastChanges).
    pub fn sync_containers(&mut self, world: &World) {
        for i in 0..self.guests.len() {
            let Some(menu) = self.guests[i]
                .menu
                .as_ref()
                .filter(|_| self.here(&self.guests[i]))
            else {
                continue;
            };
            let (size, window, slots) = (menu.size, menu.window, menu.slots);
            if !slots.valid(world) {
                self.close_container_at(i);
                continue;
            }
            for slot in 0..size + 36 {
                let now = self.view_slot(i, world, slot);
                let sent = self.guests[i]
                    .menu
                    .as_ref()
                    .and_then(|m| m.sent.get(slot).cloned());
                if sent.as_ref() == Some(&now) {
                    continue;
                }
                let net = self.stack(&now);
                let g = &mut self.guests[i];
                if let Some(m) = g.menu.as_mut() {
                    m.state = (m.state + 1) & 0x7FFF;
                    let state = m.state;
                    if m.sent.len() <= slot {
                        m.sent.resize(slot + 1, None);
                    }
                    m.sent[slot] = now;
                    g.send(&C::ContainerSetSlot(play::ContainerSetSlot {
                        container: window,
                        state,
                        slot: slot as i16,
                        item: net,
                    }));
                }
            }
            // Then the data slots, as broadcastChanges does.
            self.sync_container_data(i, world);
        }
    }
}

/// Inventory index of the n-th player slot of a container menu (27 main, then the hotbar).
fn player_slot(n: usize) -> usize {
    if n < 27 { 9 + n } else { n - 27 }
}

/// `Level.soundSeedGenerator.nextLong` for `ClientboundSoundPacket.seed`.
fn next_sound_seed(g: &mut impl RandomSource) -> i64 {
    g.next_long()
}

#[cfg(test)]
mod tests {
    use super::next_sound_seed;
    use horizon_core::random::LegacyRandom;

    #[test]
    fn two_sounds_in_one_tick_get_different_seeds() {
        let mut g = LegacyRandom::new(0);
        let a = next_sound_seed(&mut g);
        let b = next_sound_seed(&mut g);
        assert_ne!(a, b);
        assert_ne!(a, 0);
        assert_ne!(b, 0);
    }

    #[test]
    fn the_tick_is_not_the_sound_seed() {
        let ticks = 12i64;
        let mut g = LegacyRandom::new(0);
        assert_ne!(next_sound_seed(&mut g), ticks);
    }
}
