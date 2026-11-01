//! server.properties (DedicatedServerProperties over Settings): every key with
//! vanilla's default, parser and serializer. Reading a value writes its
//! normalized form back, so storing afterwards rewrites the file the way the
//! vanilla server does on start; the legacy keys are dropped.
use super::{
    java_properties::{Properties, date_string, now_millis},
    json::{self, Json},
};
use horizon_core::random::java_string_hash;
use horizon_protocol::crypto::md5;

/// MinecraftServer.ServerResourcePackInfo.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourcePackInfo {
    pub id: u128,
    pub url: String,
    pub hash: String,
    pub required: bool,
    /// The prompt component, as JSON, when it parsed.
    pub prompt: Option<Json>,
}

/// Every value DedicatedServerProperties reads, in declaration order.
#[derive(Clone, Debug)]
pub struct ServerProperties {
    pub online_mode: bool,
    pub prevent_proxy_connections: bool,
    pub server_ip: String,
    pub allow_flight: bool,
    pub motd: String,
    pub code_of_conduct: bool,
    pub bug_report_link: String,
    pub force_game_mode: bool,
    pub enforce_whitelist: bool,
    /// Difficulty id (0 peaceful .. 3 hard).
    pub difficulty: i32,
    /// GameType id (0 survival .. 3 spectator).
    pub game_mode: i32,
    pub level_name: String,
    pub server_port: i32,
    pub management_server_enabled: bool,
    pub management_server_host: String,
    pub management_server_port: i32,
    pub management_server_secret: String,
    pub management_server_tls_enabled: bool,
    pub management_server_tls_keystore: String,
    pub management_server_tls_keystore_password: String,
    pub management_server_allowed_origins: String,
    pub announce_player_achievements: Option<bool>,
    pub enable_query: bool,
    pub query_port: i32,
    pub enable_rcon: bool,
    pub rcon_port: i32,
    pub rcon_password: String,
    pub hardcore: bool,
    pub use_native_transport: bool,
    pub spawn_protection: i32,
    pub op_permission_level: i32,
    pub function_permission_level: i32,
    pub max_tick_time: i64,
    pub max_chained_neighbor_updates: i32,
    pub rate_limit: i32,
    pub command_spam_threshold_seconds: i32,
    pub chat_spam_threshold_seconds: i32,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub max_players: i32,
    pub network_compression_threshold: i32,
    pub broadcast_rcon_to_ops: bool,
    pub broadcast_console_to_ops: bool,
    pub max_world_size: i32,
    pub sync_chunk_writes: bool,
    pub region_file_compression: String,
    pub enable_jmx_monitoring: bool,
    pub enable_status: bool,
    pub hide_online_players: bool,
    pub entity_broadcast_range_percentage: i32,
    pub text_filtering_config: String,
    pub text_filtering_version: i32,
    pub player_idle_timeout: i32,
    pub status_heartbeat_interval: i32,
    pub white_list: bool,
    pub enforce_secure_profile: bool,
    pub log_ips: bool,
    pub pause_when_empty_seconds: i32,
    pub accepts_transfers: bool,
    pub level_seed: String,
    pub seed: i64,
    /// WorldOptions.randomSeed was used (empty level-seed).
    pub random_seed: bool,
    pub generate_structures: bool,
    pub generator_settings: Json,
    pub level_type: String,
    pub resource_pack: Option<ResourcePackInfo>,
    pub initial_enabled_packs: Vec<String>,
    pub initial_disabled_packs: Vec<String>,
    /// The properties as they are stored back.
    pub raw: Properties,
    /// What vanilla logs while reading (resource pack warnings).
    pub warnings: Vec<String>,
}

pub const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];
pub const GAME_MODES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];

/// Sources of the values vanilla draws at random.
pub trait Entropy {
    /// RandomSource.create().nextLong() (WorldOptions.randomSeed).
    fn next_long(&mut self) -> i64;
    /// SecureRandom.nextInt(bound).
    fn next_int(&mut self, bound: u32) -> u32;
}
/// Entropy from the operating system (/dev/urandom), else the clock.
pub struct SystemEntropy(u64);
impl Default for SystemEntropy {
    fn default() -> Self {
        let mut seed = [0u8; 8];
        let ok = std::fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut seed))
            .is_ok();
        let mut s = u64::from_le_bytes(seed);
        if !ok || s == 0 {
            s = now_millis() as u64 ^ 0x9E37_79B9_7F4A_7C15;
        }
        Self(s)
    }
}
impl SystemEntropy {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}
impl Entropy for SystemEntropy {
    fn next_long(&mut self) -> i64 {
        self.next() as i64
    }
    fn next_int(&mut self, bound: u32) -> u32 {
        (self.next() % u64::from(bound)) as u32
    }
}

/// SecurityConfig.generateSecretKey.
pub fn generate_secret_key(entropy: &mut dyn Entropy) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    (0..40)
        .map(|_| char::from(ALPHABET[entropy.next_int(ALPHABET.len() as u32) as usize]))
        .collect()
}

/// UUID.nameUUIDFromBytes.
pub fn name_uuid(bytes: &[u8]) -> u128 {
    let mut h = md5(bytes);
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    u128::from_be_bytes(h)
}

/// UUID.fromString (the lenient JDK parser: five dash-separated hex groups).
pub fn parse_uuid(s: &str) -> Option<u128> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 || s.len() > 36 {
        return None;
    }
    let mut v = [0u64; 5];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 16 {
            return None;
        }
        v[i] = u64::from_str_radix(p, 16).ok()?;
    }
    let msb = (v[0] & 0xffff_ffff) << 32 | (v[1] & 0xffff) << 16 | (v[2] & 0xffff);
    let lsb = (v[3] & 0xffff) << 48 | (v[4] & 0xffff_ffff_ffff);
    Some(u128::from(msb) << 64 | u128::from(lsb))
}

/// Boolean.valueOf.
fn java_bool(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

struct Reader {
    p: Properties,
}
impl Reader {
    fn raw(&self, key: &str) -> Option<String> {
        self.p.get(key).map(str::to_owned)
    }
    fn get<V>(
        &mut self,
        key: &str,
        de: impl Fn(&str) -> Option<V>,
        ser: impl Fn(&V) -> String,
        default: V,
    ) -> V {
        let v = self.raw(key).and_then(|s| de(&s)).unwrap_or(default);
        self.p.put(key, ser(&v));
        v
    }
    fn string(&mut self, key: &str, default: &str) -> String {
        self.get(
            key,
            |s| Some(s.to_owned()),
            String::clone,
            default.to_owned(),
        )
    }
    fn int(&mut self, key: &str, default: i32) -> i32 {
        self.get(key, |s| s.parse().ok(), i32::to_string, default)
    }
    fn long(&mut self, key: &str, default: i64) -> i64 {
        self.get(key, |s| s.parse().ok(), i64::to_string, default)
    }
    fn boolean(&mut self, key: &str, default: bool) -> bool {
        self.get(key, |s| Some(java_bool(s)), bool::to_string, default)
    }
    fn legacy(&mut self, key: &str) -> Option<String> {
        self.p.remove(key)
    }
    fn permission(&mut self, key: &str, default: i32) -> i32 {
        self.get(
            key,
            |s| s.parse::<i32>().ok().map(|v| v.clamp(0, 4)),
            i32::to_string,
            default,
        )
    }
}

impl ServerProperties {
    /// DedicatedServerProperties.fromFile; a file that cannot be read is an
    /// empty set (Settings.loadFromFile logs and continues).
    pub fn from_file(path: &std::path::Path, entropy: &mut dyn Entropy) -> Result<Self, String> {
        let p = match std::fs::read(path) {
            Ok(bytes) => Properties::load_file(&bytes).map_err(|e| e.to_string())?,
            Err(_) => Properties::default(),
        };
        Self::new(p, entropy)
    }

    /// The constructor: field initializers, then the constructor body.
    pub fn new(properties: Properties, entropy: &mut dyn Entropy) -> Result<Self, String> {
        let mut r = Reader { p: properties };
        let mut warnings = Vec::new();
        let online_mode = r.boolean("online-mode", true);
        let prevent_proxy_connections = r.boolean("prevent-proxy-connections", false);
        let server_ip = r.string("server-ip", "");
        let allow_flight = r.boolean("allow-flight", false);
        let motd = r.string("motd", "A Minecraft Server");
        let code_of_conduct = r.boolean("enable-code-of-conduct", false);
        let bug_report_link = r.string("bug-report-link", "");
        let force_game_mode = r.boolean("force-gamemode", false);
        let enforce_whitelist = r.boolean("enforce-whitelist", false);
        let difficulty = r.get(
            "difficulty",
            |s| match s.parse::<i32>() {
                Ok(id) => Some(id.rem_euclid(4)),
                Err(_) => DIFFICULTIES.iter().position(|n| *n == s).map(|i| i as i32),
            },
            |d| DIFFICULTIES[*d as usize].to_owned(),
            1,
        );
        let game_mode = r.get(
            "gamemode",
            |s| match s.parse::<i32>() {
                Ok(id) => Some(if (0..4).contains(&id) { id } else { 0 }),
                Err(_) => Some(GAME_MODES.iter().position(|n| *n == s).unwrap_or(0) as i32),
            },
            |g| GAME_MODES[*g as usize].to_owned(),
            0,
        );
        let level_name = r.string("level-name", "world");
        let server_port = r.int("server-port", 25565);
        let management_server_enabled = r.boolean("management-server-enabled", false);
        let management_server_host = r.string("management-server-host", "localhost");
        let management_server_port = r.int("management-server-port", 0);
        let secret = generate_secret_key(entropy);
        let management_server_secret = r.string("management-server-secret", &secret);
        let management_server_tls_enabled = r.boolean("management-server-tls-enabled", true);
        let management_server_tls_keystore = r.string("management-server-tls-keystore", "");
        let management_server_tls_keystore_password =
            r.string("management-server-tls-keystore-password", "");
        let management_server_allowed_origins = r.string("management-server-allowed-origins", "");
        let announce_player_achievements = r
            .legacy("announce-player-achievements")
            .map(|s| java_bool(&s));
        let enable_query = r.boolean("enable-query", false);
        let query_port = r.int("query.port", 25565);
        let enable_rcon = r.boolean("enable-rcon", false);
        let rcon_port = r.int("rcon.port", 25575);
        let rcon_password = r.string("rcon.password", "");
        let hardcore = r.boolean("hardcore", false);
        let use_native_transport = r.boolean("use-native-transport", true);
        let spawn_protection = r.int("spawn-protection", 16);
        let op_permission_level = r.permission("op-permission-level", 4);
        let function_permission_level = r.permission("function-permission-level", 2);
        let max_tick_time = r.long("max-tick-time", 60_000);
        let max_chained_neighbor_updates = r.int("max-chained-neighbor-updates", 1_000_000);
        let rate_limit = r.int("rate-limit", 0);
        let command_spam_threshold_seconds = r.int("command-spam-threshold-seconds", 10);
        let chat_spam_threshold_seconds = r.int("chat-spam-threshold-seconds", 10);
        let view_distance = r.int("view-distance", 10);
        let simulation_distance = r.int("simulation-distance", 10);
        let max_players = r.int("max-players", 20);
        let network_compression_threshold = r.int("network-compression-threshold", 256);
        let broadcast_rcon_to_ops = r.boolean("broadcast-rcon-to-ops", true);
        let broadcast_console_to_ops = r.boolean("broadcast-console-to-ops", true);
        let max_world_size = r.get(
            "max-world-size",
            |s| s.parse::<i32>().ok().map(|v| v.clamp(1, 29_999_984)),
            i32::to_string,
            29_999_984,
        );
        let sync_chunk_writes = r.boolean("sync-chunk-writes", true);
        let region_file_compression = r.string("region-file-compression", "deflate");
        let enable_jmx_monitoring = r.boolean("enable-jmx-monitoring", false);
        let enable_status = r.boolean("enable-status", true);
        let hide_online_players = r.boolean("hide-online-players", false);
        // Its deserializer is not wrapped: a non-number throws out of the constructor.
        let entity_broadcast_range_percentage = match r.raw("entity-broadcast-range-percentage") {
            Some(s) => s.parse::<i32>().map(|v| v.clamp(10, 1000)).map_err(|_| {
                format!("java.lang.NumberFormatException: For input string: \"{s}\"")
            })?,
            None => 100,
        };
        r.p.put(
            "entity-broadcast-range-percentage",
            entity_broadcast_range_percentage.to_string(),
        );
        let text_filtering_config = r.string("text-filtering-config", "");
        let text_filtering_version = r.int("text-filtering-version", 0);
        let player_idle_timeout = r.int("player-idle-timeout", 0);
        let status_heartbeat_interval = r.int("status-heartbeat-interval", 0);
        let white_list = r.boolean("white-list", true);
        let enforce_secure_profile = r.boolean("enforce-secure-profile", true);
        let log_ips = r.boolean("log-ips", true);
        let pause_when_empty_seconds = r.int("pause-when-empty-seconds", 60);
        let accepts_transfers = r.boolean("accepts-transfers", false);
        // The constructor body.
        let level_seed = r.string("level-seed", "");
        let generate_structures = r.boolean("generate-structures", true);
        let (seed, random_seed) = match parse_seed(&level_seed) {
            Some(s) => (s, false),
            None => (entropy.next_long(), true),
        };
        let generator_settings = match r.raw("generator-settings") {
            Some(s) => {
                let text = if s.is_empty() { "{}" } else { s.as_str() };
                match json::parse(text, false) {
                    Ok(v @ Json::Object(_)) => v,
                    Ok(_) => return Err("com.google.gson.JsonSyntaxException: Expected a com.google.gson.JsonObject".into()),
                    Err(e) => return Err(format!("com.google.gson.JsonParseException: {e}")),
                }
            }
            None => Json::Object(Vec::new()),
        };
        r.p.put("generator-settings", generator_settings.compact());
        let level_type = r.get(
            "level-type",
            |s| Some(s.to_lowercase()),
            String::clone,
            "minecraft:normal".to_owned(),
        );
        let pack_id = r.string("resource-pack-id", "");
        let pack_url = r.string("resource-pack", "");
        let pack_sha1 = r.string("resource-pack-sha1", "");
        let pack_hash = r.legacy("resource-pack-hash");
        let pack_required = r.boolean("require-resource-pack", false);
        let pack_prompt = r.string("resource-pack-prompt", "");
        let resource_pack = server_pack_info(
            &pack_id,
            &pack_url,
            &pack_sha1,
            pack_hash.as_deref(),
            pack_required,
            &pack_prompt,
            &mut warnings,
        );
        let enabled = r.string("initial-enabled-packs", "vanilla");
        let disabled = r.string("initial-disabled-packs", "");
        let split = |s: &str| {
            s.split(',')
                .map(|p| p.trim().to_owned())
                .collect::<Vec<_>>()
        };
        Ok(Self {
            online_mode,
            prevent_proxy_connections,
            server_ip,
            allow_flight,
            motd,
            code_of_conduct,
            bug_report_link,
            force_game_mode,
            enforce_whitelist,
            difficulty,
            game_mode,
            level_name,
            server_port,
            management_server_enabled,
            management_server_host,
            management_server_port,
            management_server_secret,
            management_server_tls_enabled,
            management_server_tls_keystore,
            management_server_tls_keystore_password,
            management_server_allowed_origins,
            announce_player_achievements,
            enable_query,
            query_port,
            enable_rcon,
            rcon_port,
            rcon_password,
            hardcore,
            use_native_transport,
            spawn_protection,
            op_permission_level,
            function_permission_level,
            max_tick_time,
            max_chained_neighbor_updates,
            rate_limit,
            command_spam_threshold_seconds,
            chat_spam_threshold_seconds,
            view_distance,
            simulation_distance,
            max_players,
            network_compression_threshold,
            broadcast_rcon_to_ops,
            broadcast_console_to_ops,
            max_world_size,
            sync_chunk_writes,
            region_file_compression,
            enable_jmx_monitoring,
            enable_status,
            hide_online_players,
            entity_broadcast_range_percentage,
            text_filtering_config,
            text_filtering_version,
            player_idle_timeout,
            status_heartbeat_interval,
            white_list,
            enforce_secure_profile,
            log_ips,
            pause_when_empty_seconds,
            accepts_transfers,
            level_seed,
            seed,
            random_seed,
            generate_structures,
            generator_settings,
            level_type,
            resource_pack,
            initial_enabled_packs: split(&enabled),
            initial_disabled_packs: split(&disabled),
            raw: r.p,
            warnings,
        })
    }

    /// Settings.store: the file the server writes back.
    pub fn stored(&self, millis: i64) -> String {
        self.raw.store(
            Some("Minecraft server properties"),
            &date_string(millis),
            false,
        )
    }
    pub fn store(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.stored(now_millis()))
    }

    /// MutableValue.update: change one key and re-read everything (the
    /// random defaults are drawn again only for keys still absent, which
    /// never happens after the first read).
    pub fn update(&self, key: &str, value: String) -> Result<Self, String> {
        let mut p = self.raw.clone();
        p.put(key, value);
        Self::new(p, &mut SystemEntropy::default())
    }
}

/// WorldOptions.parseSeed: empty is none, a long is itself, anything else
/// hashes as a Java string.
pub fn parse_seed(s: &str) -> Option<i64> {
    let s = s.trim_matches(|c: char| c <= ' ');
    if s.is_empty() {
        return None;
    }
    Some(
        s.parse::<i64>()
            .unwrap_or_else(|_| i64::from(java_string_hash(s.encode_utf16()))),
    )
}

fn server_pack_info(
    id: &str,
    url: &str,
    sha1: &str,
    legacy_hash: Option<&str>,
    required: bool,
    prompt: &str,
    warnings: &mut Vec<String>,
) -> Option<ResourcePackInfo> {
    if url.is_empty() {
        return None;
    }
    let hash = if !sha1.is_empty() {
        if legacy_hash.is_some_and(|h| !h.is_empty()) {
            warnings.push("resource-pack-hash is deprecated and found along side resource-pack-sha1. resource-pack-hash will be ignored.".into());
        }
        sha1.to_owned()
    } else if let Some(h) = legacy_hash.filter(|h| !h.is_empty()) {
        warnings.push(
            "resource-pack-hash is deprecated. Please use resource-pack-sha1 instead.".into(),
        );
        h.to_owned()
    } else {
        String::new()
    };
    if hash.is_empty() {
        warnings.push("You specified a resource pack without providing a sha1 hash. Pack will be updated on the client only if you change the name of the pack.".into());
    } else if !(hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit())) {
        warnings.push("Invalid sha1 for resource-pack-sha1".into());
    }
    let prompt = parse_prompt(prompt, warnings);
    let id = if id.is_empty() {
        let uuid = name_uuid(url.as_bytes());
        warnings.push(format!(
            "resource-pack-id missing, using default of {}",
            horizon_protocol::crypto::uuid_string(uuid)
        ));
        uuid
    } else {
        match parse_uuid(id) {
            Some(u) => u,
            None => {
                warnings.push(format!("Failed to parse '{id}' into UUID"));
                return None;
            }
        }
    };
    Some(ResourcePackInfo {
        id,
        url: url.to_owned(),
        hash,
        required,
        prompt,
    })
}

/// DedicatedServerProperties.parseResourcePackPrompt: strict JSON decoded as
/// a component (a string, an array of components, or an object with content).
fn parse_prompt(prompt: &str, warnings: &mut Vec<String>) -> Option<Json> {
    if prompt.is_empty() {
        return None;
    }
    match json::parse(prompt, true) {
        Ok(v) if is_component(&v) => Some(v),
        Ok(_) => {
            warnings.push(format!("Failed to parse resource pack prompt '{prompt}'"));
            None
        }
        Err(_) => {
            warnings.push(format!("Failed to parse resource pack prompt '{prompt}'"));
            None
        }
    }
}
fn is_component(v: &Json) -> bool {
    match v {
        Json::String(_) | Json::Number(_) | Json::Bool(_) => true,
        Json::Array(items) => !items.is_empty() && items.iter().all(is_component),
        Json::Object(m) => [
            "text",
            "translate",
            "score",
            "selector",
            "keybind",
            "nbt",
            "object",
        ]
        .iter()
        .any(|k| m.iter().any(|(n, _)| n == k)),
        Json::Null => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java_properties::line_separator;
    struct Fixed;
    impl Entropy for Fixed {
        fn next_long(&mut self) -> i64 {
            7
        }
        fn next_int(&mut self, _: u32) -> u32 {
            0
        }
    }
    #[test]
    fn defaults_match_vanilla_init_settings() {
        let p = ServerProperties::new(Properties::default(), &mut Fixed).unwrap();
        // Lines end in System.lineSeparator() (java_properties checks that): read them as "\n".
        let stored = p.stored(0).replace(line_separator(), "\n");
        assert!(stored.starts_with(
            "#Minecraft server properties\n#Thu Jan 01 00:00:00 UTC 1970\naccepts-transfers=false\n"
        ));
        assert!(stored.contains("\nlevel-type=minecraft\\:normal\n"));
        assert!(
            stored
                .contains("\nmanagement-server-secret=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n")
        );
        assert!(stored.ends_with("\nview-distance=10\nwhite-list=true\n"));
        assert_eq!(stored.lines().count(), 71);
        assert!(p.random_seed);
        assert_eq!(p.seed, 7);
    }
    #[test]
    fn parses_like_vanilla() {
        let mut raw = Properties::default();
        raw.put("difficulty", "-1".into());
        raw.put("gamemode", "bogus".into());
        raw.put("op-permission-level", "9".into());
        raw.put("level-seed", "hello".into());
        raw.put("level-type", "FLAT".into());
        raw.put("announce-player-achievements", "true".into());
        let p = ServerProperties::new(raw, &mut Fixed).unwrap();
        assert_eq!(
            (p.difficulty, p.game_mode, p.op_permission_level),
            (3, 0, 4)
        );
        assert_eq!(p.seed, 99_162_322);
        assert_eq!(p.level_type, "flat");
        assert_eq!(p.announce_player_achievements, Some(true));
        assert!(p.raw.get("announce-player-achievements").is_none());
        assert_eq!(
            parse_uuid("1-2-3-4-5"),
            Some(0x0000_0001_0002_0003_0004_0000_0000_0005)
        );
    }
}
