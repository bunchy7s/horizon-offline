//! The ban notices `Gui.addInitialScreens` queues before the title screen:
//! `Minecraft.multiplayerBan()` (the account's MULTIPLAYER ban from the services
//! attributes call), the forced name change and the banned skin
//! (`ProfileResult.actions()`), and the text `BanNoticeScreens` shows for each.
//! The network calls are in `auth.rs`; this file parses what they return.
use crate::text::Text;

/// `CommonLinks.SUSPENSION_HELP`.
pub const SUSPENSION_HELP: &str = "https://aka.ms/mcjavamoderation";

/// authlib's `BanDetails` (the id is never shown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BanDetails {
    /// Epoch milliseconds the ban ends; `None` is permanent.
    pub expires_ms: Option<i64>,
    pub reason: Option<String>,
    pub reason_message: Option<String>,
}

/// authlib's `UserProperties`, as far as the title screen reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserProperties {
    /// `UserFlag.SERVERS_ALLOWED` (`privileges.multiplayerServer.enabled`).
    pub servers_allowed: bool,
    /// `bannedScopes.get("MULTIPLAYER")`.
    pub multiplayer_ban: Option<BanDetails>,
}
impl Default for UserProperties {
    /// `UserApiService.OFFLINE_PROPERTIES`: what a failed fetch falls back to.
    fn default() -> Self {
        Self {
            servers_allowed: true,
            multiplayer_ban: None,
        }
    }
}

/// authlib's `ProfileResult.actions()`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileActions {
    pub forced_name_change: bool,
    pub using_banned_skin: bool,
}

/// What the title screen learns about the account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountStatus {
    pub properties: UserProperties,
    pub actions: ProfileActions,
}
impl AccountStatus {
    /// `Minecraft.allowsMultiplayer` (without the `--disableMultiplayer` flag).
    pub fn allows_multiplayer(&self) -> bool {
        self.properties.servers_allowed
            && self.properties.multiplayer_ban.is_none()
            && !self.actions.forced_name_change
    }
}

/// One of the screens `Gui.addInitialScreens` adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// `BanNoticeScreens.create`.
    Multiplayer(BanDetails),
    /// `BanNoticeScreens.createNameBan` with the profile's name.
    Name(String),
    /// `BanNoticeScreens.createSkinBan`.
    Skin,
}

/// `Gui.addInitialScreens`' ban part, in its order: the multiplayer ban, the
/// name ban, the skin ban.
pub fn initial_notices(status: &AccountStatus, profile_name: &str) -> Vec<Notice> {
    let mut notices = Vec::new();
    if let Some(ban) = &status.properties.multiplayer_ban {
        notices.push(Notice::Multiplayer(ban.clone()));
    }
    if status.actions.forced_name_change {
        notices.push(Notice::Name(profile_name.to_owned()));
    }
    if status.actions.using_banned_skin {
        notices.push(Notice::Skin);
    }
    notices
}

impl Notice {
    /// The screen's title (bold).
    pub fn title(&self) -> Text {
        let key = match self {
            Self::Multiplayer(ban) if ban.expires_ms.is_some() => "gui.banned.title.temporary",
            Self::Multiplayer(_) => "gui.banned.title.permanent",
            Self::Name(_) => "gui.banned.name.title",
            Self::Skin => "gui.banned.skin.title",
        };
        Text::translate(key, &[]).bold()
    }
    /// The screen's message; `now_ms` is `Instant.now()`.
    pub fn message(&self, now_ms: i64) -> Text {
        let link = || Text::literal(SUSPENSION_HELP);
        match self {
            Self::Multiplayer(ban) => Text::translate(
                "gui.banned.description",
                &[reason_text(ban), status_text(ban, now_ms), link()],
            ),
            Self::Name(name) => Text::translate(
                "gui.banned.name.description",
                &[Text::colored(name.clone(), YELLOW), link()],
            ),
            Self::Skin => Text::translate("gui.banned.skin.description", &[link()]),
        }
    }
}

/// `ChatFormatting.YELLOW`.
const YELLOW: u32 = 0xFFFF55;

/// `BanReason.byId`: the translation key suffix.
fn ban_reason(id: i32) -> Option<&'static str> {
    Some(match id {
        2 => "false_reporting",
        5 => "hate_speech",
        16 | 25 => "hate_terrorism_notorious_figure",
        17 | 19 | 23 | 31 => "generic_violation",
        21 => "harassment_or_bullying",
        27 => "defamation_impersonation_false_information",
        28 => "drugs",
        29 => "fraud",
        30 => "spam_or_advertising",
        32 => "nudity_or_pornography",
        33 | 35 | 36 => "sexually_inappropriate",
        34 => "extreme_violence_or_gore",
        53 => "imminent_harm_to_person_or_property",
        _ => return None,
    })
}

/// `BanNoticeScreens.getBanReasonText`.
fn reason_text(ban: &BanDetails) -> Text {
    let numeric = ban
        .reason
        .as_deref()
        .filter(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()));
    let Some(reason) = numeric else {
        return Text::translate("gui.banned.description.unknownreason", &[]);
    };
    // Integer.parseInt: past i32 the vanilla screen would throw; show the code as is.
    let id = reason.parse::<i32>().ok();
    let text = if let Some(key) = id.and_then(ban_reason) {
        Text::translate(&format!("gui.banned.reason.{key}"), &[]).bold()
    } else if let Some(message) = &ban.reason_message {
        Text::translate(
            "gui.banned.description.reason_id_message",
            &[Text::literal(reason), Text::literal(message.clone())],
        )
        .bold()
    } else {
        Text::translate("gui.banned.description.reason_id", &[Text::literal(reason)]).bold()
    };
    Text::translate("gui.banned.description.reason", &[text])
}

/// `BanNoticeScreens.getBanStatusText`.
fn status_text(ban: &BanDetails, now_ms: i64) -> Text {
    match ban.expires_ms {
        Some(expires) => {
            let duration = Text::translate(
                "gui.banned.description.temporary.duration",
                &[duration_text(expires - now_ms)],
            )
            .bold();
            Text::translate("gui.banned.description.temporary", &[duration])
        }
        None => Text::translate("gui.banned.description.permanent", &[]).bold(),
    }
}

/// `BanNoticeScreens.getBanDurationText`: days past 72 hours, else hours,
/// else minutes (`Duration.toHours` and friends truncate toward zero).
fn duration_text(millis: i64) -> Text {
    let count = |n: i64| Text::literal(n.to_string());
    let hours = millis / 3_600_000;
    if hours > 72 {
        Text::translate("gui.days", &[count(millis / 86_400_000)])
    } else if hours < 1 {
        Text::translate("gui.minutes", &[count(millis / 60_000)])
    } else {
        Text::translate("gui.hours", &[count(hours)])
    }
}

/// `Instant` as authlib's gson adapter reads it: an ISO-8601 string, or epoch milliseconds.
fn parse_instant(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.timestamp_millis())
            .or_else(|| s.parse().ok()),
        _ => None,
    }
}

/// `UserAttributesResponse` into `UserProperties` (`fetchProperties`). Bad JSON
/// is an error, which vanilla answers with the offline properties.
pub fn parse_properties(body: &str) -> Option<UserProperties> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let servers_allowed = v
        .pointer("/privileges/multiplayerServer/enabled")
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    let multiplayer_ban = v
        .pointer("/banStatus/bannedScopes/MULTIPLAYER")
        .filter(|b| b.is_object())
        .map(|b| {
            let text = |k: &str| b.get(k).and_then(|s| s.as_str()).map(str::to_owned);
            BanDetails {
                expires_ms: b.get("expires").and_then(parse_instant),
                reason: text("reason"),
                reason_message: text("reasonMessage"),
            }
        });
    Some(UserProperties {
        servers_allowed,
        multiplayer_ban,
    })
}

/// `MinecraftProfilePropertiesResponse.profileActions` into `ProfileActionType`s.
pub fn parse_profile_actions(body: &str) -> ProfileActions {
    let mut actions = ProfileActions::default();
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    for action in v
        .get("profileActions")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        match action.get("type").and_then(|t| t.as_str()) {
            Some("FORCED_NAME_CHANGE") => actions.forced_name_change = true,
            Some("USING_BANNED_SKIN") => actions.using_banned_skin = true,
            _ => {}
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    /// 2026-11-01T00:00:00Z.
    const EXPIRES: i64 = 1_793_491_200_000;

    #[test]
    fn parses_a_multiplayer_ban() {
        let body = r#"{"privileges":{"multiplayerServer":{"enabled":false}},
            "banStatus":{"bannedScopes":{"MULTIPLAYER":{"banId":"6b0f0a0e-0000-4000-8000-000000000000",
            "expires":"2026-11-01T00:00:00Z","reason":"21","reasonMessage":"x"}}}}"#;
        let p = parse_properties(body).unwrap();
        assert!(!p.servers_allowed);
        let ban = p.multiplayer_ban.unwrap();
        assert_eq!(ban.expires_ms, Some(EXPIRES));
        assert_eq!(ban.reason.as_deref(), Some("21"));
        let numeric =
            parse_properties(r#"{"banStatus":{"bannedScopes":{"MULTIPLAYER":{"expires":5}}}}"#)
                .unwrap();
        assert_eq!(numeric.multiplayer_ban.unwrap().expires_ms, Some(5));
        assert!(parse_properties("nope").is_none());
        let clean = parse_properties(
            r#"{"privileges":{"multiplayerServer":{"enabled":true}},"banStatus":{"bannedScopes":{}}}"#,
        )
        .unwrap();
        assert!(clean.servers_allowed && clean.multiplayer_ban.is_none());
    }

    #[test]
    fn parses_profile_actions() {
        let a = parse_profile_actions(
            r#"{"id":"x","profileActions":[{"type":"FORCED_NAME_CHANGE"},{"type":"OTHER"}]}"#,
        );
        assert!(a.forced_name_change && !a.using_banned_skin);
        let a = parse_profile_actions(r#"{"profileActions":[{"type":"USING_BANNED_SKIN"}]}"#);
        assert!(!a.forced_name_change && a.using_banned_skin);
        assert_eq!(parse_profile_actions("{}"), ProfileActions::default());
    }

    #[test]
    fn queues_in_vanilla_order() {
        let ban = BanDetails {
            expires_ms: None,
            reason: None,
            reason_message: None,
        };
        let status = AccountStatus {
            properties: UserProperties {
                servers_allowed: true,
                multiplayer_ban: Some(ban.clone()),
            },
            actions: ProfileActions {
                forced_name_change: true,
                using_banned_skin: true,
            },
        };
        assert_eq!(
            initial_notices(&status, "Steve"),
            vec![
                Notice::Multiplayer(ban),
                Notice::Name("Steve".into()),
                Notice::Skin
            ]
        );
        assert!(!status.allows_multiplayer());
        assert!(initial_notices(&AccountStatus::default(), "Steve").is_empty());
        assert!(AccountStatus::default().allows_multiplayer());
        let no_servers = AccountStatus {
            properties: UserProperties {
                servers_allowed: false,
                multiplayer_ban: None,
            },
            ..Default::default()
        };
        assert!(!no_servers.allows_multiplayer());
    }

    /// The message's component JSON: the keys and arguments, whatever language is loaded.
    fn json(notice: &Notice, now: i64) -> String {
        notice.message(now).component().to_string()
    }

    #[test]
    fn ban_text_follows_banned_screens() {
        let temp = |reason: Option<&str>, message: Option<&str>| {
            Notice::Multiplayer(BanDetails {
                expires_ms: Some(100 * HOUR),
                reason: reason.map(str::to_owned),
                reason_message: message.map(str::to_owned),
            })
        };
        assert_eq!(
            temp(None, None).title().translation_key(),
            Some("gui.banned.title.temporary")
        );
        let perm = Notice::Multiplayer(BanDetails {
            expires_ms: None,
            reason: Some("5".into()),
            reason_message: None,
        });
        assert_eq!(
            perm.title().translation_key(),
            Some("gui.banned.title.permanent")
        );
        let text = json(&perm, 0);
        assert!(text.contains("gui.banned.reason.hate_speech"));
        assert!(text.contains("gui.banned.description.permanent"));
        assert!(text.contains(SUSPENSION_HELP));
        // Unknown ids show their code, with the message when there is one.
        let with_message = json(&temp(Some("999"), Some("why")), 0);
        assert!(with_message.contains("gui.banned.description.reason_id_message"));
        assert!(with_message.contains("why"));
        assert!(json(&temp(Some("999"), None), 0).contains("gui.banned.description.reason_id\""));
        assert!(json(&temp(Some("abc"), None), 0).contains("gui.banned.description.unknownreason"));
        // 100 hours left is past 72: days (4); less is hours; under an hour is minutes.
        let duration = |now| json(&temp(None, None), now);
        assert!(duration(0).contains("gui.days") && duration(0).contains("[4]"));
        assert!(duration(60 * HOUR).contains("gui.hours") && duration(60 * HOUR).contains("[40]"));
        let minutes = duration(100 * HOUR - 90_000);
        assert!(minutes.contains("gui.minutes") && minutes.contains("[1]"));
        assert!(json(&Notice::Name("Bob".into()), 0).contains("Bob"));
        assert!(json(&Notice::Skin, 0).contains(SUSPENSION_HELP));
    }
}
