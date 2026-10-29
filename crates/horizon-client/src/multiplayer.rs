//! Multiplayer screens on the title: the server list (JoinMultiplayerScreen,
//! stored in servers.dat), Add/Edit Server, Direct Connection, and the
//! Disconnected screen. Entries are pinged in the background like
//! ServerStatusPinger to show the MOTD, player count and latency.
use crate::hud::{HudAtlas, HudBatch, rgba};
use crate::lan::discovery::Scanner;
use crate::text::{DARK_GRAY, GRAY, Text, format_translation};
use crate::title::confirm_top;
use horizon_protocol::servers::{self, ServerEntry};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const SERVER_NAME: crate::clipboard::FieldId = ("server.name", 0);
const SERVER_ADDRESS: crate::clipboard::FieldId = ("server.address", 0);
const SERVER_DIRECT: crate::clipboard::FieldId = ("server.direct", 0);
const FRIEND_CODE: crate::clipboard::FieldId = ("friend.code", 0);
const FRIEND_RELAY: crate::clipboard::FieldId = ("friend.relay", 0);

/// The result of pinging one address.
#[derive(Clone, Debug)]
pub enum Ping {
    Pinging,
    Ok {
        /// The description component, styled (`sanitizeDescription`).
        motd: Text,
        /// Online and max players; None when the status has no `players`.
        players: Option<(i64, i64)>,
        /// `ServerData.playerList`: shown over the player count.
        player_list: Vec<Text>,
        ms: u128,
        version: String,
    },
    Failed(String),
}
impl Ping {
    /// ServerStatusPinger.formatPlayerCount: dark grey with grey numbers,
    /// or `multiplayer.status.unknown` without a player count.
    pub fn player_count(players: Option<(i64, i64)>) -> Text {
        match players {
            Some((online, max)) => Text::translate(
                "multiplayer.status.player_count",
                &[
                    Text::colored(online.to_string(), GRAY),
                    Text::colored(max.to_string(), GRAY),
                ],
            ),
            None => Text::translate("multiplayer.status.unknown", &[]),
        }
        .with_color(DARK_GRAY)
    }
}
/// ServerStatusPinger: each sample name as a literal line (its `§` codes
/// styled as the font does), then `multiplayer.status.and_more` for the
/// players the sample leaves out.
fn player_list(players: &serde_json::Value, online: i64) -> Vec<Text> {
    let Some(sample) = players["sample"].as_array().filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    let mut lines: Vec<Text> = sample
        .iter()
        .map(|p| Text::literal(p["name"].as_str().unwrap_or_default()).resolve_legacy())
        .collect();
    let shown = sample.len() as i64;
    if shown < online {
        lines.push(Text::translate(
            "multiplayer.status.and_more",
            &[Text::literal((online - shown).to_string())],
        ));
    }
    lines
}
type Pings = Arc<Mutex<HashMap<String, Ping>>>;
/// Server icons by address as 64x64 RGBA (FaviconTexture), from servers.dat
/// and then from each ping's favicon.
type Icons = Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>;

#[derive(Clone, Debug)]
pub enum Screen {
    List {
        servers: Vec<ServerEntry>,
        selected: Option<usize>,
        pings: Pings,
        icons: Icons,
        /// Worlds announced on the local network, listed after the servers.
        lan: Scanner,
        scroll: crate::ui::Scroll,
    },
    /// Add (index None) or edit a server entry.
    Edit {
        index: Option<usize>,
        name: String,
        address: String,
        focus: u8,
        /// Server Resource Packs (`acceptTextures`): Enabled, Disabled, or
        /// none for Prompt.
        packs: Option<bool>,
    },
    Direct {
        address: String,
    },
    /// ConfirmScreen for Delete: the list it returns to, the server's name
    /// and how many lines the warning took when last drawn.
    ConfirmDelete {
        list: Box<Screen>,
        name: String,
        lines: std::cell::Cell<usize>,
    },
    /// Join a friend's world with its invite code (Horizon's own, behind the
    /// title screen's Friends button).
    Friend {
        code: String,
        /// The relay for short codes (optional; saved on Join).
        relay: String,
        focus: u8,
    },
    Disconnected {
        reason: String,
        /// Auto-reconnect's next attempt, until it is cancelled.
        retry: Option<crate::reconnect::Retry>,
    },
}
/// What a click asks the title screen to do.
pub enum Outcome {
    None,
    Back,
    Connect(String),
}
/// A button: x, y, width, label, enabled, id.
pub type ButtonSpec = (f32, f32, f32, String, bool, u8);

/// `this.width / 2` (or `height / 2`): `Screen`'s size is an int, so the
/// division drops the half and widgets stay on whole GUI pixels. A half
/// pixel lands between device pixels and draws thinner borders and uneven
/// glyphs at most window heights.
fn half(size: f32) -> f32 {
    (size.ceil() / 2.0).floor()
}
/// `this.height / 4`, as an int division.
fn quarter(size: f32) -> f32 {
    (size.ceil() / 4.0).floor()
}
/// `centeredText` around `width / 2`: `x - font.width(text) / 2`.
fn centered_x(width: f32, text_width: f32) -> f32 {
    half(width) - half(text_width)
}

pub fn servers_file(saves: &Path) -> PathBuf {
    saves
        .parent()
        .map_or_else(|| PathBuf::from("servers.dat"), |p| p.join("servers.dat"))
}
/// `FaviconTexture.upload`: a PNG of exactly 64x64 pixels, else no icon.
fn icon_pixels(png: &[u8]) -> Option<Arc<Vec<u8>>> {
    crate::hud::decode_bytes(png)
        .ok()
        .filter(|(w, h, _)| *w == 64 && *h == 64)
        .map(|(_, _, p)| Arc::new(p))
}
/// `ServerStatus.Favicon.CODEC`: a base64 PNG behind the data URL prefix.
fn favicon_bytes(json: &serde_json::Value) -> Option<Vec<u8>> {
    let url = json["favicon"].as_str()?;
    let base64 = url
        .strip_prefix("data:image/png;base64,")?
        .replace('\n', "");
    crate::skins::base64_decode(&base64)
}
fn ping_all(servers: &[ServerEntry], pings: &Pings, icons: &Icons, file: &Path) {
    for s in servers {
        let address = s.ip.clone();
        pings.lock().unwrap().insert(address.clone(), Ping::Pinging);
        let saved = s.icon.clone();
        if let Some(pixels) = saved
            .as_deref()
            .and_then(crate::skins::base64_decode)
            .and_then(|png| icon_pixels(&png))
        {
            icons.lock().unwrap().insert(address.clone(), pixels);
        }
        let (pings, icons, file) = (pings.clone(), icons.clone(), file.to_owned());
        let _ = std::thread::Builder::new()
            .name("horizon-ping".into())
            .spawn(move || {
                let result = match crate::net::ping(&address, Duration::from_secs(5)) {
                    Ok((json, ms)) => {
                        let motd = crate::component::to_text(&json["description"], &mut |s| {
                            Text::literal(s)
                        });
                        let players = json["players"].as_object().map(|p| {
                            (
                                p.get("online").and_then(|n| n.as_i64()).unwrap_or(0),
                                p.get("max").and_then(|n| n.as_i64()).unwrap_or(0),
                            )
                        });
                        let player_list = players
                            .map(|(online, _)| player_list(&json["players"], online))
                            .unwrap_or_default();
                        let version = json["version"]["name"].as_str().unwrap_or("?").to_owned();
                        // A new favicon replaces the saved one (an invalid one
                        // clears it) and servers.dat is saved again.
                        if let Some(png) = favicon_bytes(&json) {
                            let encoded = crate::lan::base64(&png);
                            let pixels = icon_pixels(&png);
                            match &pixels {
                                Some(p) => icons.lock().unwrap().insert(address.clone(), p.clone()),
                                None => icons.lock().unwrap().remove(&address),
                            };
                            let icon = pixels.map(|_| encoded);
                            if icon != saved {
                                save_icon(&file, &address, icon);
                            }
                        }
                        Ping::Ok {
                            motd,
                            players,
                            player_list,
                            ms,
                            version,
                        }
                    }
                    Err(e) => Ping::Failed(e),
                };
                pings.lock().unwrap().insert(address, result);
            });
    }
}
/// Save the list after a move or delete, keeping the icons pings stored in
/// servers.dat since it was loaded, and the Code of Conduct accepted on a server.
fn save_servers(file: &Path, servers: &mut [ServerEntry]) {
    let _guard = SAVING.lock();
    let saved = servers::load(file);
    for entry in servers.iter_mut() {
        if let Some(s) = saved.iter().find(|s| s.ip == entry.ip) {
            entry.icon = s.icon.clone();
            entry.accepted_code_of_conduct = s.accepted_code_of_conduct;
        }
    }
    let _ = servers::save(file, servers);
}
/// Held while servers.dat is read and written back.
static SAVING: Mutex<()> = Mutex::new(());
/// Store `icon` on the entries for `address` (OnlineServerEntry.updateServerList).
fn save_icon(file: &Path, address: &str, icon: Option<String>) {
    let _guard = SAVING.lock();
    let mut list = servers::load(file);
    let mut changed = false;
    for entry in list.iter_mut().filter(|e| e.ip == address) {
        changed |= entry.icon != icon;
        entry.icon = icon.clone();
    }
    if changed {
        let _ = servers::save(file, &list);
    }
}

/// `ClientPacketListener.handleServerData`: a valid icon the joined server
/// sends replaces the one its list entries keep.
pub fn save_server_icon(saves: &Path, address: &str, png: &[u8]) {
    if icon_pixels(png).is_some() {
        save_icon(&servers_file(saves), address, Some(crate::lan::base64(png)));
    }
}

/// Horizon's official survival server: a name and an address like any entry.
pub const OFFICIAL_SERVER: (&str, &str) = ("Horizon Official", "horizon.bunchy7s.dev");
/// Beside servers.dat once the official server was offered, so a player who
/// deletes it never gets it back. The launcher writes the same file.
const OFFICIAL_MARKER: &str = "horizon_official_server.txt";

/// Puts the official server at the top of the list in `game_dir`, once: not
/// again after the player deletes it, and not when the list has its address.
pub fn add_official_server(game_dir: &Path) -> std::io::Result<()> {
    let marker = game_dir.join(OFFICIAL_MARKER);
    if marker.exists() {
        return Ok(());
    }
    let (name, address) = OFFICIAL_SERVER;
    let file = game_dir.join("servers.dat");
    let key = |ip: &str| servers::parse_address(ip).map(|(h, p)| (h.to_ascii_lowercase(), p));
    let _guard = SAVING.lock();
    if !servers::load(&file)
        .iter()
        .any(|s| key(&s.ip) == key(address))
    {
        servers::prepend(&file, &ServerEntry::new(name, address))?;
    }
    std::fs::write(marker, format!("{address}\n"))
}

/// The address of list row `index`: the saved servers, then the LAN games.
fn entry_address(servers: &[ServerEntry], lan: &Scanner, index: usize) -> Option<String> {
    match servers.get(index) {
        Some(s) => Some(s.ip.clone()),
        None => lan
            .games()
            .into_iter()
            .nth(index - servers.len())
            .map(|g| g.address),
    }
}

/// The Server Resource Packs button's label: `CommonComponents.optionNameValue`.
fn pack_setting_label(packs: Option<bool>) -> String {
    let (key, fallback) = match packs {
        Some(true) => ("manageServer.resourcePack.enabled", "Enabled"),
        Some(false) => ("manageServer.resourcePack.disabled", "Disabled"),
        None => ("manageServer.resourcePack.prompt", "Prompt"),
    };
    let name = crate::lang::get_or("manageServer.resourcePack", "Server Resource Packs");
    crate::lang::get_or("options.generic_value", "%s: %s")
        .replacen("%s", &name, 1)
        .replacen("%s", &crate::lang::get_or(key, fallback), 1)
}

/// JoinMultiplayerScreen's `HeaderAndFooterLayout(33, 60)`.
const HEADER: f32 = 33.0;
const FOOTER: f32 = 60.0;
/// ServerSelectionList: rows 305 wide and 36 high.
const ROW_WIDTH: f32 = 305.0;
const ROW_HEIGHT: f32 = 36.0;
/// One row of the list: the saved servers, the LAN header, the LAN games.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Server(usize),
    LanHeader,
    Lan(usize),
}
/// The list's top and bottom and its rows' left edge (`getRowLeft`).
fn list_area(w: f32, h: f32) -> (f32, f32, f32) {
    (
        HEADER,
        h - FOOTER,
        (w / 2.0).floor() - (ROW_WIDTH / 2.0).floor(),
    )
}
fn rows(servers: usize, lan: usize) -> impl Iterator<Item = Row> {
    (0..servers)
        .map(Row::Server)
        .chain(std::iter::once(Row::LanHeader))
        .chain((0..lan).map(Row::Lan))
}
/// `contentHeight`: the rows and 4 pixels of padding.
fn content_height(servers: usize, lan: usize) -> f32 {
    (servers + 1 + lan) as f32 * ROW_HEIGHT + 4.0
}
/// The row under `(x, y)` (`getEntryAtPosition`), scrolled by `scroll`.
fn row_at(
    servers: usize,
    lan: usize,
    (x, y): (f32, f32),
    size: (f32, f32),
    scroll: f32,
) -> Option<Row> {
    let (top, bottom, left) = list_area(size.0, size.1);
    if y < top || y >= bottom || x < left || x >= left + ROW_WIDTH {
        return None;
    }
    let index = ((y - top - 2.0 + scroll.trunc()) / ROW_HEIGHT).floor();
    if index < 0.0 {
        return None;
    }
    rows(servers, lan).nth(index as usize)
}
/// SelectableEntry's hover areas over the 32-pixel icon, relative to it.
fn over_right_half(x: f32, y: f32) -> bool {
    (16.0..32.0).contains(&x) && (0.0..32.0).contains(&y)
}
fn over_top_left(x: f32, y: f32) -> bool {
    (0.0..16.0).contains(&x) && (0.0..16.0).contains(&y)
}
fn over_bottom_left(x: f32, y: f32) -> bool {
    (0.0..16.0).contains(&x) && (16.0..32.0).contains(&y)
}
/// The server's favicon in icon slot `slot`, else `misc/unknown_server`.
fn draw_server_icon(
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    pixels: Option<&[u8]>,
    slot: usize,
    x: f32,
    y: f32,
) {
    let sprite = format!("horizon/world-icon{}", slot % crate::hud::WORLD_ICON_SLOTS);
    if let Some(pixels) = pixels
        && let Some(rect) = atlas.sprites.get(&sprite)
    {
        batch.patches.push(crate::font::Patch {
            x: (rect.u0 * atlas.width as f32).round() as u32,
            y: (rect.v0 * atlas.height as f32).round() as u32,
            width: 64,
            height: 64,
            rgba: pixels.to_vec(),
        });
        batch.sprite_scaled(atlas, &sprite, x, y, 32.0, 32.0, u32::MAX);
    } else {
        batch.sprite_scaled(atlas, "misc/unknown_server", x, y, 32.0, 32.0, u32::MAX);
    }
}
/// `LoadingDotsText.get`.
fn loading_dots(millis: u128) -> &'static str {
    ["O o o", "o O o", "o o O", "o O o"][(millis / 300 % 4) as usize]
}

fn ping_tooltip(
    servers: &[ServerEntry],
    pings: &HashMap<String, Ping>,
    size: (f32, f32),
    scroll: f32,
    cursor: (f32, f32),
) -> Option<Text> {
    let Row::Server(i) = row_at(servers.len(), 0, cursor, size, scroll)? else {
        return None;
    };
    let (top, _, left) = list_area(size.0, size.1);
    let x = left + 2.0 + ROW_WIDTH - 4.0 - 15.0;
    let y = top + 4.0 - scroll.trunc() + i as f32 * ROW_HEIGHT;
    if cursor.0 < x || cursor.0 >= x + 10.0 || cursor.1 < y || cursor.1 >= y + 8.0 {
        return None;
    }
    Some(match pings.get(&servers[i].ip) {
        Some(Ping::Ok { ms, .. }) => format_translation(
            &crate::lang::get_or("multiplayer.status.ping", "%s ms"),
            &[Text::literal(ms.to_string())],
        ),
        Some(Ping::Failed(_)) => Text::literal(crate::lang::get_or(
            "multiplayer.status.no_connection",
            "(no connection)",
        )),
        _ => Text::literal(crate::lang::get_or(
            "multiplayer.status.pinging",
            "Pinging...",
        )),
    })
}
/// OnlineServerEntry's player list, over the player count drawn left of
/// the status icon; `width` measures that count.
fn player_list_tooltip(
    servers: &[ServerEntry],
    pings: &HashMap<String, Ping>,
    size: (f32, f32),
    scroll: f32,
    cursor: (f32, f32),
    width: impl Fn(&Text) -> f32,
) -> Option<Vec<Text>> {
    let Row::Server(i) = row_at(servers.len(), 0, cursor, size, scroll)? else {
        return None;
    };
    let Some(Ping::Ok {
        players,
        player_list,
        ..
    }) = pings.get(&servers[i].ip)
    else {
        return None;
    };
    let (top, _, left) = list_area(size.0, size.1);
    let icon_x = left + 2.0 + ROW_WIDTH - 4.0 - 15.0;
    let y = top + 4.0 - scroll.trunc() + i as f32 * ROW_HEIGHT;
    let w = width(&Ping::player_count(*players)).ceil();
    let x = icon_x - w - 5.0;
    if cursor.0 < x || cursor.0 >= x + w || cursor.1 < y || cursor.1 >= y + 8.0 {
        return None;
    }
    (!player_list.is_empty()).then(|| player_list.clone())
}

impl Screen {
    /// Draw after the list and footer so the tooltip is neither clipped nor covered.
    pub fn draw_tooltip(&self, atlas: &HudAtlas, batch: &mut HudBatch, cursor: (f32, f32)) {
        let Self::List {
            servers,
            pings,
            lan,
            scroll,
            ..
        } = self
        else {
            return;
        };
        let size = (batch.width.ceil(), batch.height.ceil());
        let (top, bottom, _) = list_area(size.0, size.1);
        let mut scroll = *scroll;
        scroll.clamp(
            top,
            bottom,
            content_height(servers.len(), lan.games().len()),
        );
        let pings = pings.lock().unwrap();
        let lines = match ping_tooltip(servers, &pings, size, scroll.amount, cursor) {
            Some(text) => vec![text],
            None => player_list_tooltip(servers, &pings, size, scroll.amount, cursor, |t| {
                atlas.font.width_text(t)
            })
            .unwrap_or_default(),
        };
        crate::ui::tooltip(batch, atlas, &lines, cursor);
    }

    pub fn list(saves: &Path) -> Self {
        let servers = servers::load(&servers_file(saves));
        let pings: Pings = Arc::default();
        let icons: Icons = Arc::default();
        ping_all(&servers, &pings, &icons, &servers_file(saves));
        Self::List {
            servers,
            selected: None,
            pings,
            icons,
            lan: Scanner::start(),
            scroll: crate::ui::Scroll::default(),
        }
    }
    pub fn buttons(&self, w: f32, h: f32) -> Vec<ButtonSpec> {
        let b = |x: f32, y: f32, bw: f32, label: &str, enabled: bool, id: u8| {
            (x, y, bw, label.to_owned(), enabled, id)
        };
        match self {
            // JoinMultiplayerScreen's footer: two rows of buttons, 4 apart,
            // centred in the 60-pixel footer. Join needs a server or LAN game
            // selected (onSelectedChange), Edit and Delete a saved server.
            Self::List {
                servers, selected, ..
            } => {
                let (w, h) = (w.ceil(), h.ceil());
                let x = ((w - 308.0) / 2.0).floor();
                let saved = selected.is_some_and(|i| i < servers.len());
                let label = |key: &str, fallback: &str| crate::lang::get_or(key, fallback);
                vec![
                    b(
                        x,
                        h - 52.0,
                        100.0,
                        &label("selectServer.select", "Join Server"),
                        selected.is_some(),
                        40,
                    ),
                    b(
                        x + 104.0,
                        h - 52.0,
                        100.0,
                        &label("selectServer.direct", "Direct Connection"),
                        true,
                        41,
                    ),
                    b(
                        x + 208.0,
                        h - 52.0,
                        100.0,
                        &label("selectServer.add", "Add Server"),
                        true,
                        42,
                    ),
                    b(
                        x,
                        h - 28.0,
                        74.0,
                        &label("selectServer.edit", "Edit"),
                        saved,
                        43,
                    ),
                    b(
                        x + 78.0,
                        h - 28.0,
                        74.0,
                        &label("selectServer.delete", "Delete"),
                        saved,
                        44,
                    ),
                    b(
                        x + 156.0,
                        h - 28.0,
                        74.0,
                        &label("selectServer.refresh", "Refresh"),
                        true,
                        45,
                    ),
                    b(
                        x + 234.0,
                        h - 28.0,
                        74.0,
                        &label("gui.back", "Back"),
                        true,
                        46,
                    ),
                ]
            }
            Self::Edit { address, packs, .. } => vec![
                // ManageServerScreen's CycleButton, above Done.
                b(
                    half(w) - 100.0,
                    quarter(h) + 72.0,
                    200.0,
                    &pack_setting_label(*packs),
                    true,
                    60,
                ),
                b(
                    half(w) - 100.0,
                    quarter(h) + 96.0 + 18.0,
                    200.0,
                    &crate::lang::get_or("gui.done", "Done"),
                    servers::parse_address(address).is_some(),
                    50,
                ),
                b(
                    half(w) - 100.0,
                    quarter(h) + 120.0 + 18.0,
                    200.0,
                    &crate::lang::get_or("gui.cancel", "Cancel"),
                    true,
                    51,
                ),
            ],
            // ConfirmScreen's two 150-wide buttons, 4 apart, under the message.
            Self::ConfirmDelete { lines, .. } => {
                let y = confirm_top(h, lines.get()) + 41.0 + lines.get() as f32 * 9.0;
                let x = (w.ceil() / 2.0).floor();
                vec![
                    b(
                        x - 152.0,
                        y,
                        150.0,
                        &crate::lang::get_or("selectServer.deleteButton", "Delete"),
                        true,
                        58,
                    ),
                    b(
                        x + 2.0,
                        y,
                        150.0,
                        &crate::lang::get_or("gui.cancel", "Cancel"),
                        true,
                        59,
                    ),
                ]
            }
            Self::Direct { address } => vec![
                b(
                    half(w) - 100.0,
                    quarter(h) + 96.0 + 12.0,
                    200.0,
                    &crate::lang::get_or("selectServer.select", "Join Server"),
                    servers::parse_address(address).is_some(),
                    52,
                ),
                b(
                    half(w) - 100.0,
                    quarter(h) + 120.0 + 12.0,
                    200.0,
                    &crate::lang::get_or("gui.cancel", "Cancel"),
                    true,
                    53,
                ),
            ],
            Self::Friend { code, .. } => vec![
                b(
                    half(w) - 100.0,
                    quarter(h) + 96.0 + 18.0,
                    200.0,
                    "Join",
                    horizon_friends::invite::parse(code).is_some(),
                    56,
                ),
                b(
                    half(w) - 100.0,
                    quarter(h) + 120.0 + 18.0,
                    200.0,
                    &crate::lang::get_or("gui.cancel", "Cancel"),
                    true,
                    57,
                ),
            ],
            Self::Disconnected { retry, .. } => {
                let mut v = vec![b(
                    half(w) - 100.0,
                    half(h) + 30.0,
                    200.0,
                    &crate::lang::get_or("gui.toMenu", "Back to Server List"),
                    true,
                    54,
                )];
                if retry.is_some() {
                    v.push(b(
                        half(w) - 100.0,
                        half(h) + 54.0,
                        200.0,
                        "Cancel Reconnecting",
                        true,
                        55,
                    ));
                }
                v
            }
        }
    }
    /// An EditBox and its label: the label at `label_y` (one pixel in, in
    /// 0xA0A0A0), the 200x20 box at `box_y`. The box is the same `EditBox`
    /// the world screens edit (`Caret`): cursor, selection and scrolling.
    fn field(
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut crate::ui::Caret,
        focused_at: Instant,
        label: &str,
        value: &str,
        id: crate::clipboard::FieldId,
        max: usize,
        (label_y, box_y): (f32, f32),
        focused: bool,
    ) {
        let x = (batch.width.ceil() / 2.0).floor() - 100.0;
        batch.text(
            atlas,
            label,
            x + 1.0,
            label_y,
            rgba(160, 160, 160, 255),
            true,
        );
        caret.draw(
            batch,
            atlas,
            (id, value, max),
            [x, box_y, 200.0, 20.0],
            focused,
            focused_at,
        );
    }
    pub fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut crate::ui::Caret,
        focused_at: Instant,
        cursor: (f32, f32),
    ) {
        let (w, h) = (batch.width, batch.height);
        let max = self.max_length();
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        // GuiGraphicsExtractor.centeredText at width / 2: x - width(text) / 2.
        let centered = |batch: &mut HudBatch, text: &str, y: f32, color: u32| {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, centered_x(w, tw), y, color, true);
        };
        match self {
            Self::List {
                servers,
                selected,
                pings,
                icons,
                lan,
                scroll,
            } => {
                let (w, h) = (w.ceil(), h.ceil());
                let title = crate::lang::get_or("multiplayer.title", "Play Multiplayer");
                // The header's StringWidget, centred in its 33 pixels.
                let tw = HudBatch::text_width(atlas, &title);
                let tx = ((w - tw.ceil()) / 2.0).floor();
                batch.text(
                    atlas,
                    &title,
                    tx,
                    ((HEADER - 9.0) / 2.0).floor(),
                    u32::MAX,
                    true,
                );
                let games = lan.games();
                let (top, bottom, left) = list_area(w, h);
                let content = content_height(servers.len(), games.len());
                let mut scroll = *scroll;
                scroll.clamp(top, bottom, content);
                // AbstractSelectionList.extractWidgetRenderState: background,
                // the rows clipped to the list, separators, scrollbar.
                crate::ui::list_background(batch, atlas, top, bottom, false);
                let first = batch.vertices.len();
                let pings = pings.lock().unwrap();
                let icons = icons.lock().unwrap();
                // The row under the pointer (`isHovered`).
                let hovered = row_at(servers.len(), games.len(), cursor, (w, h), scroll.amount);
                let millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis());
                let gray = rgba(128, 128, 128, 255);
                for (i, row) in rows(servers.len(), games.len()).enumerate() {
                    let y = top + 2.0 - scroll.amount.trunc() + i as f32 * ROW_HEIGHT;
                    if y + ROW_HEIGHT < top || y > bottom {
                        continue;
                    }
                    let index = match row {
                        Row::Server(k) => Some(k),
                        Row::Lan(k) => Some(servers.len() + k),
                        Row::LanHeader => None,
                    };
                    if index.is_some()
                        && y + ROW_HEIGHT / 2.0 > top
                        && y + ROW_HEIGHT / 2.0 < bottom
                    {
                        let rect = [left, y, ROW_WIDTH, ROW_HEIGHT];
                        // The control socket's widget tree names the row.
                        let name = match row {
                            Row::Server(k) => servers[k].name.as_str(),
                            Row::Lan(k) => games[k].motd.as_str(),
                            Row::LanHeader => "",
                        };
                        crate::widgets::record_as(name, rect, true, crate::widgets::Kind::Row);
                        crate::widgets::target_as(rect, crate::widgets::Kind::Row);
                    }
                    // extractSelection: a white outline round black (the list
                    // has focus once a row is clicked).
                    if index.is_some() && *selected == index {
                        batch.fill(atlas, left, y, ROW_WIDTH, ROW_HEIGHT, u32::MAX);
                        batch.fill(
                            atlas,
                            left + 1.0,
                            y + 1.0,
                            ROW_WIDTH - 2.0,
                            ROW_HEIGHT - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    // Entry content: 2 pixels in from the row.
                    let (cx, cy) = (left + 2.0, y + 2.0);
                    match row {
                        Row::Server(k) => {
                            let server = &servers[k];
                            let icon = icons.get(&server.ip);
                            draw_server_icon(atlas, batch, icon.map(|p| p.as_slice()), i, cx, cy);
                            batch.text(atlas, &server.name, cx + 35.0, cy + 1.0, u32::MAX, true);
                            // OnlineServerEntry: up to two lines of MOTD, the
                            // player count left of the status icon, all with
                            // GuiGraphics.text's shadow.
                            let (motd, status, sprite) = match pings.get(&server.ip) {
                                Some(Ping::Ok {
                                    motd, players, ms, ..
                                }) => (
                                    motd.resolve_legacy(),
                                    Ping::player_count(*players),
                                    format!(
                                        "gui/sprites/server_list/ping_{}",
                                        match ms {
                                            0..150 => 5,
                                            150..300 => 4,
                                            300..600 => 3,
                                            600..1000 => 2,
                                            _ => 1,
                                        }
                                    ),
                                ),
                                Some(Ping::Failed(_)) => (
                                    Text::translate("multiplayer.status.cannot_connect", &[])
                                        .with_color(0xFF0000),
                                    Text::empty(),
                                    "gui/sprites/server_list/unreachable".to_owned(),
                                ),
                                _ => {
                                    // The pinging bars bounce, offset by the row.
                                    let frame = (millis / 100 + i as u128 * 2) & 7;
                                    let frame = if frame > 4 { 8 - frame } else { frame };
                                    (
                                        Text::translate("multiplayer.status.pinging", &[]),
                                        Text::empty(),
                                        format!("gui/sprites/server_list/pinging_{}", frame + 1),
                                    )
                                }
                            };
                            for (n, line) in atlas
                                .font
                                .split(&motd, ROW_WIDTH - 4.0 - 32.0 - 2.0)
                                .iter()
                                .take(2)
                                .enumerate()
                            {
                                batch.text_styled(
                                    atlas,
                                    line,
                                    cx + 35.0,
                                    cy + 12.0 + 9.0 * n as f32,
                                    gray,
                                    true,
                                );
                            }
                            let icon_x = cx + ROW_WIDTH - 4.0 - 15.0;
                            batch.sprite_scaled(atlas, &sprite, icon_x, cy, 10.0, 8.0, u32::MAX);
                            let sw = atlas.font.width_text(&status).ceil();
                            batch.text_styled(
                                atlas,
                                &status,
                                icon_x - sw - 5.0,
                                cy + 1.0,
                                gray,
                                true,
                            );
                            // Hovering the row: the join arrow over the icon
                            // and the move arrows, each highlighted under
                            // the pointer.
                            if hovered == Some(row) {
                                let (rx, ry) = (cursor.0 - cx, cursor.1 - cy);
                                batch.fill(atlas, cx, cy, 32.0, 32.0, 0xA0909090);
                                for (sprite, shown, over) in [
                                    ("join", true, over_right_half(rx, ry)),
                                    ("move_up", k > 0, over_top_left(rx, ry)),
                                    ("move_down", k + 1 < servers.len(), over_bottom_left(rx, ry)),
                                ] {
                                    if shown {
                                        let lit = if over { "_highlighted" } else { "" };
                                        batch.sprite_scaled(
                                            atlas,
                                            &format!("gui/sprites/server_list/{sprite}{lit}"),
                                            cx,
                                            cy,
                                            32.0,
                                            32.0,
                                            u32::MAX,
                                        );
                                    }
                                }
                            }
                        }
                        Row::LanHeader => {
                            // LANHeader's LoadingDotsWidget: the label, then
                            // the dots on its last line, both centred.
                            let label = crate::lang::get_or(
                                "lanServer.scanning",
                                "Scanning for games on your local network",
                            );
                            let middle = cx + ((ROW_WIDTH - 4.0) / 2.0).floor();
                            let lw = HudBatch::text_width(atlas, &label).ceil();
                            let lx = middle - (lw / 2.0).floor();
                            let centre = lx + (lw / 2.0).floor();
                            batch.text(atlas, &label, lx, cy + 2.0, u32::MAX, false);
                            let dots = loading_dots(millis);
                            let dw = HudBatch::text_width(atlas, dots).ceil();
                            batch.text(
                                atlas,
                                dots,
                                centre - (dw / 2.0).floor(),
                                cy + 28.0 - 9.0 - 2.0,
                                gray,
                                false,
                            );
                        }
                        Row::Lan(k) => {
                            let game = &games[k];
                            let header = crate::lang::get_or("lanServer.title", "LAN World");
                            batch.text(atlas, &header, cx + 35.0, cy + 1.0, u32::MAX, false);
                            batch.text(atlas, &game.motd, cx + 35.0, cy + 12.0, gray, false);
                            batch.text(atlas, &game.address, cx + 35.0, cy + 23.0, gray, false);
                        }
                    }
                }
                batch.clip_since(first, 0.0, top, w, bottom);
                crate::ui::separators(batch, atlas, top, bottom, false);
                crate::ui::scrollbar(
                    batch,
                    atlas,
                    left + ROW_WIDTH + 8.0,
                    top,
                    bottom,
                    content,
                    scroll.amount,
                );
            }
            Self::Edit {
                index,
                name,
                address,
                focus,
                ..
            } => {
                centered(
                    batch,
                    &if index.is_some() {
                        crate::lang::get_or("manageServer.edit.title", "Edit Server Info")
                    } else {
                        crate::lang::get_or("manageServer.add.title", "Add Server")
                    },
                    17.0,
                    u32::MAX,
                );
                // ManageServerScreen: labels at 53 and 94, boxes at 66 and 106.
                Self::field(
                    atlas,
                    batch,
                    caret,
                    focused_at,
                    &crate::lang::get_or("manageServer.enterName", "Server Name"),
                    name,
                    SERVER_NAME,
                    max,
                    (53.0, 66.0),
                    *focus == 0,
                );
                Self::field(
                    atlas,
                    batch,
                    caret,
                    focused_at,
                    &crate::lang::get_or("manageServer.enterIp", "Server Address"),
                    address,
                    SERVER_ADDRESS,
                    max,
                    (94.0, 106.0),
                    *focus == 1,
                );
            }
            Self::Direct { address } => {
                // DirectJoinServerScreen: the title at 20, the label at 100
                // and the box at 116.
                centered(
                    batch,
                    &crate::lang::get_or("selectServer.direct", "Direct Connection"),
                    20.0,
                    u32::MAX,
                );
                Self::field(
                    atlas,
                    batch,
                    caret,
                    focused_at,
                    &crate::lang::get_or("manageServer.enterIp", "Server Address"),
                    address,
                    SERVER_DIRECT,
                    max,
                    (100.0, 116.0),
                    true,
                );
            }
            Self::ConfirmDelete { name, lines, .. } => {
                // ConfirmScreen: the title, then the message wrapped to
                // width - 50, centred as a block.
                let warning = format_translation(
                    &crate::lang::get_or(
                        "selectServer.deleteWarning",
                        "'%s' will be lost forever! (A long time!)",
                    ),
                    &[Text::literal(name.clone())],
                );
                let message = atlas.font.split(&warning, (w.ceil() - 50.0).max(1.0));
                lines.set(message.len());
                let top = confirm_top(h, message.len());
                centered(
                    batch,
                    &crate::lang::get_or(
                        "selectServer.deleteQuestion",
                        "Are you sure you want to remove this server?",
                    ),
                    top,
                    u32::MAX,
                );
                for (i, line) in message.iter().enumerate() {
                    centered(batch, &line.plain(), top + 17.0 + i as f32 * 9.0, u32::MAX);
                }
            }
            Self::Friend { code, relay, focus } => {
                centered(batch, "Join a Friend", 20.0, u32::MAX);
                Self::field(
                    atlas,
                    batch,
                    caret,
                    focused_at,
                    "Invite Code",
                    code,
                    FRIEND_CODE,
                    max,
                    (53.0, 64.0),
                    *focus == 0,
                );
                Self::field(
                    atlas,
                    batch,
                    caret,
                    focused_at,
                    "Relay (optional)",
                    relay,
                    FRIEND_RELAY,
                    max,
                    (94.0, 105.0),
                    *focus == 1,
                );
            }
            Self::Disconnected { reason, retry } => {
                centered(
                    batch,
                    &crate::lang::get_or("disconnect.lost", "Connection Lost"),
                    half(h) - 40.0,
                    rgba(170, 170, 170, 255),
                );
                for (i, line) in wrap(reason, 300.0, atlas).iter().enumerate() {
                    centered(batch, line, half(h) - 16.0 + i as f32 * 9.0, u32::MAX);
                }
                if let Some(r) = retry {
                    let text = r.text(std::time::Instant::now());
                    centered(batch, &text, half(h) + 16.0, rgba(170, 170, 170, 255));
                }
            }
        }
    }
    pub fn click(
        &mut self,
        id: Option<u8>,
        x: f32,
        y: f32,
        size: (f32, f32),
        saves: &Path,
    ) -> Outcome {
        let (w, h) = size;
        match (&mut *self, id) {
            (
                Self::List {
                    servers,
                    selected,
                    lan,
                    ..
                },
                Some(40),
            ) => {
                if let Some(address) = selected.and_then(|i| entry_address(servers, lan, i)) {
                    return Outcome::Connect(address);
                }
            }
            (Self::List { .. }, Some(41)) => {
                *self = Self::Direct {
                    address: String::new(),
                }
            }
            (Self::List { .. }, Some(47)) => *self = Self::friend(),
            (Self::List { .. }, Some(42)) => {
                *self = Self::Edit {
                    index: None,
                    name: crate::lang::get_or("selectServer.defaultName", "Minecraft Server"),
                    address: String::new(),
                    focus: 1,
                    packs: None,
                }
            }
            (
                Self::List {
                    servers, selected, ..
                },
                Some(43),
            ) => {
                if let Some((i, s)) = selected.and_then(|i| servers.get(i).map(|s| (i, s.clone())))
                {
                    *self = Self::Edit {
                        index: Some(i),
                        name: s.name,
                        address: s.ip,
                        focus: 0,
                        packs: s.accept_textures,
                    };
                }
            }
            (
                Self::List {
                    servers, selected, ..
                },
                Some(44),
            ) => {
                // JoinMultiplayerScreen's Delete asks first (ConfirmScreen).
                if let Some(name) = selected
                    .and_then(|i| servers.get(i))
                    .map(|s| s.name.clone())
                {
                    let list = std::mem::replace(
                        self,
                        Self::Direct {
                            address: String::new(),
                        },
                    );
                    *self = Self::ConfirmDelete {
                        list: Box::new(list),
                        name,
                        lines: std::cell::Cell::new(1),
                    };
                }
            }
            // deleteCallback: yes removes the server and clears the
            // selection; either answer returns to the list as it was.
            (Self::ConfirmDelete { .. }, Some(id @ (58 | 59))) => {
                let Self::ConfirmDelete { list, .. } = std::mem::replace(
                    self,
                    Self::Direct {
                        address: String::new(),
                    },
                ) else {
                    unreachable!()
                };
                *self = *list;
                if id == 58
                    && let Self::List {
                        servers, selected, ..
                    } = self
                    && let Some(i) = selected.take()
                    && i < servers.len()
                {
                    servers.remove(i);
                    save_servers(&servers_file(saves), servers);
                }
            }
            (Self::List { .. }, Some(45)) => *self = Self::list(saves),
            (Self::List { .. }, Some(46)) => return Outcome::Back,
            (
                Self::List {
                    servers,
                    selected,
                    lan,
                    scroll,
                    ..
                },
                None,
            ) => {
                let (w, h) = (w.ceil(), h.ceil());
                let games = lan.games().len();
                let (top, bottom, left) = list_area(w, h);
                let content = content_height(servers.len(), games);
                if scroll.press((x, y), left + ROW_WIDTH + 8.0, top, bottom, content) {
                    return Outcome::None;
                }
                let row = row_at(servers.len(), games, (x, y), (w, h), scroll.amount);
                // OnlineServerEntry.mouseClicked: the icon's right half joins,
                // its left quarters move the server up or down (and keep it
                // selected).
                if let Some(Row::Server(k)) = row {
                    let row_top = top + 2.0 - scroll.amount.trunc() + k as f32 * ROW_HEIGHT;
                    let (rx, ry) = ((x - left - 2.0).floor(), (y - row_top - 2.0).floor());
                    if over_right_half(rx, ry) {
                        *selected = Some(k);
                        return Outcome::Connect(servers[k].ip.clone());
                    }
                    let to = if k > 0 && over_top_left(rx, ry) {
                        Some(k - 1)
                    } else if k + 1 < servers.len() && over_bottom_left(rx, ry) {
                        Some(k + 1)
                    } else {
                        None
                    };
                    if let Some(to) = to {
                        servers.swap(k, to);
                        save_servers(&servers_file(saves), servers);
                        *selected = Some(to);
                        return Outcome::None;
                    }
                }
                // Clicking a selected row again joins it; the LAN header
                // takes the click without anything to join.
                let index = match row {
                    Some(Row::Server(k)) => Some(k),
                    Some(Row::Lan(k)) => Some(servers.len() + k),
                    Some(Row::LanHeader) => {
                        *selected = None;
                        None
                    }
                    None => None,
                };
                if let Some(index) = index
                    && let Some(address) = entry_address(servers, lan, index)
                {
                    if *selected == Some(index) {
                        return Outcome::Connect(address);
                    }
                    *selected = Some(index);
                }
            }
            // CycleButton: Enabled, Disabled, Prompt, and round again.
            (Self::Edit { packs, .. }, Some(60)) => {
                *packs = match *packs {
                    Some(true) => Some(false),
                    Some(false) => None,
                    None => Some(true),
                }
            }
            (
                Self::Edit {
                    index,
                    name,
                    address,
                    packs,
                    ..
                },
                Some(50),
            ) => {
                let mut list = servers::load(&servers_file(saves));
                let mut entry = ServerEntry::new(name.trim(), address.trim());
                entry.accept_textures = *packs;
                match index.and_then(|i| list.get_mut(i)) {
                    Some(e) => {
                        e.name = entry.name;
                        e.ip = entry.ip;
                        e.accept_textures = entry.accept_textures;
                    }
                    None => list.push(entry),
                }
                let _ = servers::save(&servers_file(saves), &list);
                *self = Self::list(saves);
            }
            (
                Self::Edit { .. }
                | Self::Direct { .. }
                | Self::Friend { .. }
                | Self::Disconnected { .. },
                Some(51 | 53 | 54 | 57),
            ) => *self = Self::list(saves),
            (Self::Disconnected { retry, .. }, Some(55)) => *retry = None,
            (Self::Direct { address }, Some(52)) => {
                return Outcome::Connect(direct_address(address));
            }
            (Self::Friend { code, relay, .. }, Some(56)) => {
                if horizon_friends::invite::parse(code).is_some() {
                    crate::net::save_friends_relay(relay.trim());
                    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
                    return Outcome::Connect(format!("{}{code}", crate::net::FRIEND));
                }
            }
            (Self::Edit { .. } | Self::Friend { .. }, None) => {
                let boxes = if matches!(self, Self::Edit { .. }) {
                    [66.0, 106.0]
                } else {
                    [64.0, 105.0]
                };
                let (Self::Edit { focus, .. } | Self::Friend { focus, .. }) = self else {
                    unreachable!()
                };
                for (i, fy) in [(0u8, boxes[0]), (1, boxes[1])] {
                    if y >= fy && y < fy + 20.0 && (half(w) - 100.0..half(w) + 100.0).contains(&x) {
                        *focus = i;
                    }
                }
            }
            _ => {}
        }
        Outcome::None
    }
    /// The mouse wheel over the server list (`scrollRate`: half a row).
    pub fn wheel(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
        if let Self::List {
            servers,
            lan,
            scroll,
            ..
        } = self
        {
            let (top, bottom, _) = list_area(size.0.ceil(), size.1.ceil());
            if cursor.1 >= top && cursor.1 < bottom {
                let content = content_height(servers.len(), lan.games().len());
                scroll.wheel(delta, ROW_HEIGHT / 2.0, top, bottom, content);
            }
        }
    }
    /// The pointer moved: a held scrollbar follows it.
    pub fn drag(&mut self, y: f32, size: (f32, f32)) {
        if let Self::List {
            servers,
            lan,
            scroll,
            ..
        } = self
        {
            let (top, bottom, _) = list_area(size.0.ceil(), size.1.ceil());
            scroll.drag(
                y,
                top,
                bottom,
                content_height(servers.len(), lan.games().len()),
            );
        }
    }
    pub fn release(&mut self) {
        if let Self::List { scroll, .. } = self {
            scroll.release();
        }
    }
    /// The focused box, in GUI pixels: name at 66, address at 106
    /// (`ManageServerScreen`), direct address at 116, friend fields at 64 and 105.
    pub fn focused_box(&self, (w, _h): (f32, f32)) -> Option<[f32; 4]> {
        let y = match self {
            Self::Edit { focus: 0, .. } => 66.0,
            Self::Edit { .. } => 106.0,
            Self::Direct { .. } => 116.0,
            Self::Friend { focus: 0, .. } => 64.0,
            Self::Friend { .. } => 105.0,
            _ => return None,
        };
        Some([(w / 2.0).floor() - 100.0, y, 200.0, 20.0])
    }
    fn focused_field(&mut self) -> Option<&mut String> {
        match self {
            Self::Edit {
                name,
                address,
                focus,
                ..
            } => Some(if *focus == 0 { name } else { address }),
            Self::Direct { address } => Some(address),
            Self::Friend {
                code, relay, focus, ..
            } => Some(if *focus == 0 { code } else { relay }),
            _ => None,
        }
    }
    /// Join a friend, with the relay the player set before.
    pub fn friend() -> Self {
        Self::Friend {
            code: String::new(),
            relay: crate::net::friends_relay_setting().unwrap_or_default(),
            focus: 0,
        }
    }
    /// Auto-reconnect's attempt, once its countdown is over.
    pub fn due_retry(&mut self) -> Option<crate::reconnect::Retry> {
        let Self::Disconnected { retry, .. } = self else {
            return None;
        };
        if !retry.as_ref()?.due(std::time::Instant::now()) {
            return None;
        }
        retry.take()
    }
    /// The focused text field, its id and maximum length (copy and paste).
    pub fn focused_text(&mut self) -> Option<(crate::clipboard::FieldId, &mut String, usize)> {
        let id = match self {
            Self::Edit { focus: 0, .. } => SERVER_NAME,
            Self::Edit { .. } => SERVER_ADDRESS,
            Self::Direct { .. } => SERVER_DIRECT,
            Self::Friend { focus: 0, .. } => FRIEND_CODE,
            Self::Friend { .. } => FRIEND_RELAY,
            _ => return None,
        };
        let max = self.max_length();
        self.focused_field().map(|f| (id, f, max))
    }
    /// Invite codes with IPv6 and a relay run past a server address's 128.
    fn max_length(&self) -> usize {
        if matches!(self, Self::Friend { .. }) {
            256
        } else {
            128
        }
    }
    pub fn tab(&mut self) {
        if let Self::Edit { focus, .. } | Self::Friend { focus, .. } = self {
            *focus = (*focus + 1) % 2;
        }
    }
    /// Escape: back to the list, or out of it (true).
    pub fn escape(&mut self, saves: &Path) -> bool {
        match self {
            Self::List { .. } => true,
            // ConfirmScreen.keyPressed: Escape answers no.
            Self::ConfirmDelete { list, .. } => {
                let list = std::mem::replace(
                    &mut **list,
                    Self::Direct {
                        address: String::new(),
                    },
                );
                *self = list;
                false
            }
            _ => {
                *self = Self::list(saves);
                false
            }
        }
    }
}

/// Direct Connection's address; a full invite code pasted there joins the
/// friend (a short relay code could be a host name, so it needs Join a Friend).
fn direct_address(address: &str) -> String {
    let code: String = address.chars().filter(|c| !c.is_whitespace()).collect();
    if matches!(
        horizon_friends::invite::parse(&code),
        Some(horizon_friends::invite::Parsed::Long(_))
    ) {
        format!("{}{code}", crate::net::FRIEND)
    } else {
        address.trim().to_owned()
    }
}
/// Greedy word wrap to a pixel width.
pub fn wrap(text: &str, width: f32, atlas: &HudAtlas) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if HudBatch::text_width(atlas, &candidate) > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line = word.to_owned();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_connection_joins_a_pasted_invite_and_keeps_host_names() {
        let invite = horizon_friends::invite::Invite {
            secret: *b"0123456789",
            port: 25580,
            endpoints: vec!["192.168.1.20:25580".parse().unwrap()],
            relay: None,
        }
        .encode();
        assert_eq!(
            direct_address(&format!(" {invite} ")),
            format!("{}{invite}", crate::net::FRIEND)
        );
        for host in ["localhost", "example.org:25566", "abcdefgh", "[::1]:25568"] {
            assert_eq!(direct_address(host), host);
        }
    }

    #[test]
    fn ping_tooltip_follows_only_visible_status_bars_and_current_ping() {
        let servers: Vec<_> = (0..6)
            .map(|i| ServerEntry::new("Test", &format!("server{i}")))
            .collect();
        let mut pings = HashMap::new();
        let size = (427.0, 240.0);
        let hover = |pings: &HashMap<String, Ping>, scroll, cursor| {
            ping_tooltip(&servers, pings, size, scroll, cursor).map(|t| t.plain())
        };
        // The first bar is at (349, 37), with a 10 by 8 hit area.
        assert_eq!(
            hover(&pings, 0.0, (349.0, 37.0)).as_deref(),
            Some("Pinging...")
        );
        pings.insert(
            "server0".into(),
            Ping::Ok {
                motd: Text::empty(),
                players: None,
                player_list: Vec::new(),
                ms: 42,
                version: String::new(),
            },
        );
        assert_eq!(hover(&pings, 0.0, (358.9, 44.9)).as_deref(), Some("42 ms"));
        pings.insert("server1".into(), Ping::Pinging);
        for cursor in [
            (348.9, 37.0),
            (359.0, 37.0),
            (349.0, 36.9),
            (349.0, 45.0),
            (100.0, 40.0),
            (349.0, 32.0),
            (349.0, 181.0),
        ] {
            assert!(hover(&pings, 0.0, cursor).is_none(), "{cursor:?}");
        }
        // Scrolling moves the next row's bars into the same location.
        assert_eq!(
            hover(&pings, 36.0, (349.0, 37.0)).as_deref(),
            Some("Pinging...")
        );
        assert_eq!(hover(&pings, 5.0, (349.0, 34.0)).as_deref(), Some("42 ms"));
        assert!(hover(&pings, 36.0, (349.0, 33.0)).is_none());
        assert!(hover(&pings, 0.0, (349.0, 179.0)).is_none());
        assert!(hover(&pings, 216.0, (349.0, 37.0)).is_none()); // LAN header
        pings.insert(
            "server0".into(),
            Ping::Failed("private connection error".into()),
        );
        assert_eq!(
            hover(&pings, 0.0, (349.0, 37.0)).as_deref(),
            Some("(no connection)")
        );
    }

    #[test]
    fn screens_lay_out_on_whole_gui_pixels_at_any_window_size() {
        // A 1000-pixel-high window at GUI scale 3 is 334 GUI pixels high and
        // a 1281-wide one 427 wide: vanilla's `height / 4` is 83 and
        // `width / 2` 213, never 83.5 and 213.5.
        let direct = Screen::Direct {
            address: "localhost".into(),
        };
        let at = |screen: &Screen, w, h| -> Vec<(f32, f32)> {
            screen.buttons(w, h).iter().map(|b| (b.0, b.1)).collect()
        };
        assert_eq!(at(&direct, 427.0, 334.0), [(113.0, 191.0), (113.0, 215.0)]);
        assert_eq!(at(&direct, 320.0, 240.0), [(60.0, 168.0), (60.0, 192.0)]);
        let friend = Screen::Friend {
            code: String::new(),
            relay: String::new(),
            focus: 0,
        };
        assert_eq!(at(&friend, 427.0, 253.0), [(113.0, 177.0), (113.0, 201.0)]);
        let lost = Screen::Disconnected {
            reason: String::new(),
            retry: None,
        };
        assert_eq!(at(&lost, 427.0, 253.0), [(113.0, 156.0)]);
        // centeredText: `width / 2 - font.width(text) / 2`.
        assert_eq!(centered_x(427.0, 85.0), 171.0);
        assert_eq!(centered_x(320.0, 84.0), 118.0);
    }

    #[test]
    fn player_list_styles_sample_names_and_counts_the_rest() {
        let players = serde_json::json!({
            "online": 5,
            "sample": [
                {"name": "\u{a7}b\u{a7}lBIT\u{a7}rMADE"},
                {"name": ""},
                {"name": "\u{a7}8\u{2022}\u{a7}r \u{a7}bLobby"},
            ]
        });
        let lines = player_list(&players, 5);
        assert_eq!(lines.len(), 4);
        let plain: Vec<String> = lines.iter().take(3).map(Text::plain).collect();
        assert_eq!(plain, ["BITMADE", "", "\u{2022} Lobby"]);
        assert_eq!(player_list(&players, 3).len(), 3);
        assert!(player_list(&serde_json::json!({"online": 2}), 2).is_empty());
        assert!(player_list(&serde_json::json!({"online": 2, "sample": []}), 2).is_empty());
    }

    #[test]
    fn player_list_tooltip_covers_only_the_player_count() {
        let servers = vec![ServerEntry::new("Test", "server0")];
        let size = (427.0, 240.0);
        let mut pings = HashMap::new();
        let hover = |pings: &HashMap<String, Ping>, cursor| {
            player_list_tooltip(&servers, pings, size, 0.0, cursor, |_| 20.0)
                .map(|l| l.iter().map(Text::plain).collect::<Vec<_>>())
        };
        assert!(hover(&pings, (324.0, 37.0)).is_none());
        let ok = |player_list| Ping::Ok {
            motd: Text::empty(),
            players: Some((0, 500)),
            player_list,
            ms: 1,
            version: String::new(),
        };
        pings.insert("server0".into(), ok(vec![Text::literal("Lobby")]));
        // The 20-wide count ends 5 pixels left of the bars at (349, 37).
        for cursor in [(324.0, 37.0), (343.9, 44.9)] {
            assert_eq!(hover(&pings, cursor), Some(vec!["Lobby".to_owned()]));
        }
        for cursor in [
            (323.9, 37.0),
            (344.0, 37.0),
            (324.0, 36.9),
            (324.0, 45.0),
            (349.0, 37.0),
        ] {
            assert!(hover(&pings, cursor).is_none(), "{cursor:?}");
        }
        pings.insert("server0".into(), ok(Vec::new()));
        assert!(hover(&pings, (324.0, 37.0)).is_none());
    }

    #[test]
    fn the_lan_header_follows_the_saved_servers() {
        let all: Vec<Row> = rows(2, 1).collect();
        assert_eq!(
            all,
            [Row::Server(0), Row::Server(1), Row::LanHeader, Row::Lan(0)]
        );
        // No saved servers: the list is the scanning header alone.
        assert_eq!(rows(0, 0).collect::<Vec<_>>(), [Row::LanHeader]);
        assert_eq!(content_height(0, 0), 40.0);
    }

    #[test]
    fn rows_sit_two_pixels_under_the_header_and_scroll_with_the_list() {
        // 427x240 GUI: the list spans 33..180, rows 305 wide from x 61.
        let size = (427.0, 240.0);
        assert_eq!(list_area(size.0, size.1), (33.0, 180.0, 61.0));
        assert_eq!(row_at(3, 0, (100.0, 35.0), size, 0.0), Some(Row::Server(0)));
        assert_eq!(row_at(3, 0, (100.0, 70.9), size, 0.0), Some(Row::Server(0)));
        assert_eq!(row_at(3, 0, (100.0, 71.0), size, 0.0), Some(Row::Server(1)));
        assert_eq!(
            row_at(3, 0, (100.0, 35.0), size, 36.0),
            Some(Row::Server(1))
        );
        assert_eq!(
            row_at(3, 0, (100.0, 143.0), size, 0.0),
            Some(Row::LanHeader)
        );
        assert_eq!(row_at(3, 0, (60.0, 40.0), size, 0.0), None);
        assert_eq!(row_at(3, 0, (100.0, 32.0), size, 0.0), None);
    }

    #[test]
    fn loading_dots_step_every_300_ms() {
        let frames: Vec<&str> = [0, 299, 300, 600, 900, 1200].map(loading_dots).to_vec();
        assert_eq!(
            frames,
            ["O o o", "O o o", "o O o", "o o O", "o O o", "O o o"]
        );
    }

    #[test]
    fn player_count_is_dark_gray_with_gray_numbers() {
        assert_eq!(
            Ping::player_count(Some((3, 20))).component(),
            serde_json::json!({
                "translate": "multiplayer.status.player_count",
                "with": [{"text": "3", "color": "gray"}, {"text": "20", "color": "gray"}],
                "color": "dark_gray",
            })
        );
        assert_eq!(
            Ping::player_count(None).component(),
            serde_json::json!({"translate": "multiplayer.status.unknown", "color": "dark_gray"})
        );
    }

    fn png(size: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, size, size);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![200; (size * size * 4) as usize])
            .unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn favicons_are_64_pixel_pngs_behind_the_data_url() {
        let icon = png(64);
        let encoded = crate::lan::base64(&icon);
        let (a, b) = encoded.split_at(10);
        let json = serde_json::json!({ "favicon": format!("data:image/png;base64,{a}\n{b}") });
        let bytes = favicon_bytes(&json).unwrap();
        assert_eq!(bytes, icon);
        assert_eq!(icon_pixels(&bytes).unwrap().len(), 64 * 64 * 4);
        // FaviconTexture refuses other sizes; other formats are not icons.
        assert!(icon_pixels(&png(32)).is_none());
        assert!(favicon_bytes(&serde_json::json!({ "favicon": encoded })).is_none());
    }

    #[test]
    fn the_official_server_is_added_once_at_the_top() {
        let dir = std::env::temp_dir().join(format!("horizon-mp-official-{}", std::process::id()));
        let (name, address) = OFFICIAL_SERVER;
        let file = dir.join("servers.dat");
        let ips = || -> Vec<String> { servers::load(&file).into_iter().map(|s| s.ip).collect() };
        let fresh = |list: &[ServerEntry]| {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            if !list.is_empty() {
                servers::save(&file, list).unwrap();
            }
        };
        // A fresh list: just the official server, named like it.
        fresh(&[]);
        add_official_server(&dir).unwrap();
        assert_eq!(servers::load(&file), [ServerEntry::new(name, address)]);
        // An existing list: on top, the player's servers kept in their order.
        let mine = [
            ServerEntry::new("Mine", "127.0.0.1:2"),
            ServerEntry::new("Other", "example.org"),
        ];
        fresh(&mine);
        add_official_server(&dir).unwrap();
        assert_eq!(ips(), [address, "127.0.0.1:2", "example.org"]);
        // Once only: a second start adds nothing.
        add_official_server(&dir).unwrap();
        assert_eq!(ips(), [address, "127.0.0.1:2", "example.org"]);
        // Already there under the player's own name and spelling: not duplicated.
        fresh(&[
            ServerEntry::new("SMP", "Horizon.Bunchy7s.dev:25565"),
            mine[0].clone(),
        ]);
        add_official_server(&dir).unwrap();
        assert_eq!(ips(), ["Horizon.Bunchy7s.dev:25565", "127.0.0.1:2"]);
        // Deleted by the player: never back.
        fresh(&mine);
        add_official_server(&dir).unwrap();
        servers::save(&file, &mine).unwrap();
        add_official_server(&dir).unwrap();
        assert_eq!(ips(), ["127.0.0.1:2", "example.org"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn icon_arrows_join_and_move_servers() {
        let dir = std::env::temp_dir().join(format!("horizon-mp-arrows-{}", std::process::id()));
        let saves = dir.join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        let file = servers_file(&saves);
        let names = ["a", "b", "c"];
        let list: Vec<ServerEntry> = names
            .iter()
            .map(|n| ServerEntry::new(n, &format!("127.0.0.1:{}", 1 + n.len())))
            .collect();
        servers::save(&file, &list).unwrap();
        let mut screen = Screen::list(&saves);
        let size = (427.0, 240.0);
        let order = |screen: &Screen| match screen {
            Screen::List {
                servers, selected, ..
            } => (
                servers.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
                *selected,
            ),
            _ => unreachable!(),
        };
        // 427x240: rows start at x 61, the first icon at (63, 37).
        let (ix, iy) = (63.0, 37.0);
        // The first server has no up arrow: its top-left quarter selects it.
        assert!(matches!(
            screen.click(None, ix + 4.0, iy + 4.0, size, &saves),
            Outcome::None
        ));
        assert_eq!(
            order(&screen),
            (vec!["a".into(), "b".into(), "c".into()], Some(0))
        );
        // Bottom-left quarter: down, saved, and still selected.
        screen.click(None, ix + 4.0, iy + 20.0, size, &saves);
        assert_eq!(
            order(&screen),
            (vec!["b".into(), "a".into(), "c".into()], Some(1))
        );
        let saved: Vec<String> = servers::load(&file).into_iter().map(|s| s.name).collect();
        assert_eq!(saved, ["b", "a", "c"]);
        // Top-left quarter of the second row: back up.
        screen.click(None, ix + 4.0, iy + ROW_HEIGHT + 4.0, size, &saves);
        assert_eq!(
            order(&screen),
            (vec!["a".into(), "b".into(), "c".into()], Some(0))
        );
        // The right half joins at once.
        match screen.click(None, ix + 20.0, iy + 2.0 * ROW_HEIGHT + 10.0, size, &saves) {
            Outcome::Connect(address) => assert_eq!(address, list[2].ip),
            _ => panic!("the join arrow connects"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_asks_before_removing_the_server() {
        let dir = std::env::temp_dir().join(format!("horizon-mp-delete-{}", std::process::id()));
        let saves = dir.join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        let file = servers_file(&saves);
        let list = vec![
            ServerEntry::new("Keep", "127.0.0.1:2"),
            ServerEntry::new("Gone", "127.0.0.1:3"),
        ];
        servers::save(&file, &list).unwrap();
        let size = (427.0, 240.0);
        let names =
            || -> Vec<String> { servers::load(&file).into_iter().map(|s| s.name).collect() };
        let mut screen = Screen::list(&saves);
        let select = |screen: &mut Screen| {
            if let Screen::List { selected, .. } = screen {
                *selected = Some(1);
            }
        };
        select(&mut screen);
        // Delete opens the ConfirmScreen: nothing is removed yet.
        screen.click(Some(44), 0.0, 0.0, size, &saves);
        assert!(matches!(&screen, Screen::ConfirmDelete { name, .. } if name == "Gone"));
        let labels: Vec<(f32, f32, String, u8)> = screen
            .buttons(size.0, size.1)
            .into_iter()
            .map(|b| (b.0, b.1, b.3, b.5))
            .collect();
        assert_eq!(
            labels,
            [
                (61.0, 135.0, "Delete".to_owned(), 58),
                (215.0, 135.0, "Cancel".to_owned(), 59),
            ]
        );
        assert_eq!(names(), ["Keep", "Gone"]);
        // Cancel and Escape return to the list with the selection kept.
        screen.click(Some(59), 0.0, 0.0, size, &saves);
        assert!(matches!(
            &screen,
            Screen::List {
                selected: Some(1),
                ..
            }
        ));
        screen.click(Some(44), 0.0, 0.0, size, &saves);
        assert!(!screen.escape(&saves));
        assert!(matches!(
            &screen,
            Screen::List {
                selected: Some(1),
                ..
            }
        ));
        assert_eq!(names(), ["Keep", "Gone"]);
        // Delete removes it, saves, and clears the selection.
        screen.click(Some(44), 0.0, 0.0, size, &saves);
        screen.click(Some(58), 0.0, 0.0, size, &saves);
        assert!(
            matches!(&screen, Screen::List { selected: None, servers, .. } if servers.len() == 1)
        );
        assert_eq!(names(), ["Keep"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_cycles_and_saves_the_server_resource_pack_setting() {
        let dir = std::env::temp_dir().join(format!("horizon-mp-packs-{}", std::process::id()));
        let saves = dir.join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        let file = servers_file(&saves);
        let mut entry = ServerEntry::new("Packs", "127.0.0.1:4");
        entry.accept_textures = Some(false);
        servers::save(&file, &[entry]).unwrap();
        let size = (427.0, 240.0);
        let mut screen = Screen::list(&saves);
        if let Screen::List { selected, .. } = &mut screen {
            *selected = Some(0);
        }
        screen.click(Some(43), 0.0, 0.0, size, &saves);
        let cycle = |screen: &Screen| {
            screen
                .buttons(size.0, size.1)
                .into_iter()
                .find(|b| b.5 == 60)
                .map(|b| (b.0, b.1, b.2, b.3))
        };
        // ManageServerScreen: 200 wide at width / 2 - 100 (ints: 427 / 2 is
        // 213) and height / 4 + 72, the saved value first.
        assert_eq!(
            cycle(&screen),
            Some((
                113.0,
                132.0,
                200.0,
                "Server Resource Packs: Disabled".into()
            ))
        );
        // Disabled, Prompt, Enabled: Cancel keeps what was saved.
        screen.click(Some(60), 0.0, 0.0, size, &saves);
        assert_eq!(cycle(&screen).unwrap().3, "Server Resource Packs: Prompt");
        screen.click(Some(51), 0.0, 0.0, size, &saves);
        assert_eq!(servers::load(&file)[0].accept_textures, Some(false));
        // Done saves it.
        if let Screen::List { selected, .. } = &mut screen {
            *selected = Some(0);
        }
        screen.click(Some(43), 0.0, 0.0, size, &saves);
        screen.click(Some(60), 0.0, 0.0, size, &saves);
        screen.click(Some(60), 0.0, 0.0, size, &saves);
        assert_eq!(cycle(&screen).unwrap().3, "Server Resource Packs: Enabled");
        screen.click(Some(50), 0.0, 0.0, size, &saves);
        assert_eq!(servers::load(&file)[0].accept_textures, Some(true));
        // A new server asks.
        screen.click(Some(42), 0.0, 0.0, size, &saves);
        assert_eq!(cycle(&screen).unwrap().3, "Server Resource Packs: Prompt");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
