//! Configuration state: registries, tags, known packs, feature flags and the
//! common packets (keep-alive, ping, custom payloads, resource packs).
use crate::{
    Result,
    buf::{
        Bool, Byte, Codec, Ident, Int, List, Long, Nbt, Opt, Reader, Rest, Str, StrN, UByte, Uuid,
        VarInt, Writer,
    },
    packet, packet_enum,
    packets::Empty,
};
use horizon_world::nbt::Tag;

packet!(CookieRequest { key: String => Ident });
packet!(CustomPayload { channel: String => Ident, data: Vec<u8> => Rest });
packet!(
    /// Common disconnect: a chat component (NBT).
    Disconnect { reason: Tag => Nbt }
);
packet!(KeepAlive { id: i64 => Long });
packet!(Ping { id: i32 => Int });
packet!(
    /// RegistrySynchronization.PackedRegistryEntry: data is absent when the
    /// client already has the entry from a known pack.
    RegistryEntry { id: String => Ident, data: Option<Tag> => Opt<Nbt> }
);
packet!(RegistryData { registry: String => Ident, entries: Vec<RegistryEntry> => List<RegistryEntry> });
packet!(ResourcePackPop { id: Option<u128> => Opt<Uuid> });
packet!(ResourcePackPush {
    id: u128 => Uuid,
    url: String => Str,
    hash: String => StrN<40>,
    required: bool => Bool,
    prompt: Option<Tag> => Opt<Nbt>,
});
packet!(StoreCookie { key: String => Ident, payload: Vec<u8> => crate::buf::Bytes });
packet!(Transfer { host: String => Str, port: i32 => VarInt });
packet!(UpdateEnabledFeatures { features: Vec<String> => List<Ident> });
packet!(TagEntry { name: String => Ident, ids: Vec<i32> => List<VarInt> });
packet!(RegistryTags { registry: String => Ident, tags: Vec<TagEntry> => List<TagEntry> });
packet!(
    /// ClientboundUpdateTagsPacket: tags per registry.
    UpdateTags { registries: Vec<RegistryTags> => List<RegistryTags> }
);
packet!(KnownPack { namespace: String => Str, id: String => Str, version: String => Str });
packet!(SelectKnownPacks { packs: Vec<KnownPack> => List<KnownPack> });
packet!(CodeOfConduct { text: String => Str });

/// ServerLinks.UntrustedEntry's `type` (Either<KnownLinkType, Component>): a
/// known link's id, or a custom label component.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerLinkKind {
    /// ServerLinks.KnownLinkType by id (ByIdMap: unknown ids read as 0).
    Known(i32),
    /// A custom label (TRUSTED_CONTEXT_FREE_STREAM_CODEC: a component in NBT).
    Custom(Tag),
}
/// ServerLinks.UntrustedEntry: the label and the unchecked link string (the
/// client keeps only `http`/`https` URIs, Util.parseAndValidateUntrustedUri).
#[derive(Clone, Debug, PartialEq)]
pub struct ServerLink {
    pub kind: ServerLinkKind,
    pub link: String,
}
impl Codec<ServerLink> for ServerLink {
    fn read(r: &mut Reader) -> Result<Self> {
        let kind = if Bool::read(r)? {
            ServerLinkKind::Known(VarInt::read(r)?)
        } else {
            ServerLinkKind::Custom(Nbt::read(r)?)
        };
        Ok(Self {
            kind,
            link: Str::read(r)?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        match &v.kind {
            ServerLinkKind::Known(id) => {
                Bool::write(&true, w);
                VarInt::write(id, w);
            }
            ServerLinkKind::Custom(tag) => {
                Bool::write(&false, w);
                Nbt::write(tag, w);
            }
        }
        Str::write(&v.link, w);
    }
}
packet!(
    /// ClientboundServerLinksPacket (a common packet: configuration and play).
    /// Links the pause menu's "Server Links..." screen lists.
    ServerLinks { links: Vec<ServerLink> => List<ServerLink> }
);
packet!(
    /// ClientboundShowDialogPacket.CONTEXT_FREE_STREAM_CODEC: the dialog
    /// itself (`Dialog.DIRECT_CODEC` as NBT).
    ShowDialog { dialog: Tag => Nbt }
);
packet!(
    /// ServerboundCustomClickActionPacket: a `custom` click event's id and
    /// payload (a dialog's answer).
    CustomClickAction { id: String => Ident, payload: Option<Tag> => ClickPayload }
);
/// ServerboundCustomClickActionPacket.UNTRUSTED_TAG_CODEC: an optional tag
/// (`End` when there is none) behind its length, 65536 bytes at most.
pub struct ClickPayload;
impl Codec<Option<Tag>> for ClickPayload {
    fn read(r: &mut Reader) -> Result<Option<Tag>> {
        let n = r.count(65536)?;
        let tag = Reader::new(r.bytes(n)?).nbt()?;
        Ok((tag != Tag::End).then_some(tag))
    }
    fn write(v: &Option<Tag>, w: &mut Writer) {
        let mut tag = Writer::new();
        tag.nbt(v.as_ref().unwrap_or(&Tag::End));
        w.varint(tag.data.len() as i32);
        w.bytes(&tag.data);
    }
}

packet_enum!(Clientbound, crate::ids::configuration::clientbound {
    CookieRequest(CookieRequest) = COOKIE_REQUEST,
    CustomPayload(CustomPayload) = CUSTOM_PAYLOAD,
    Disconnect(Disconnect) = DISCONNECT,
    FinishConfiguration(Empty) = FINISH_CONFIGURATION,
    KeepAlive(KeepAlive) = KEEP_ALIVE,
    Ping(Ping) = PING,
    ResetChat(Empty) = RESET_CHAT,
    RegistryData(RegistryData) = REGISTRY_DATA,
    ResourcePackPop(ResourcePackPop) = RESOURCE_PACK_POP,
    ResourcePackPush(ResourcePackPush) = RESOURCE_PACK_PUSH,
    StoreCookie(StoreCookie) = STORE_COOKIE,
    Transfer(Transfer) = TRANSFER,
    UpdateEnabledFeatures(UpdateEnabledFeatures) = UPDATE_ENABLED_FEATURES,
    UpdateTags(UpdateTags) = UPDATE_TAGS,
    SelectKnownPacks(SelectKnownPacks) = SELECT_KNOWN_PACKS,
    ClearDialog(Empty) = CLEAR_DIALOG,
    ServerLinks(ServerLinks) = SERVER_LINKS,
    ShowDialog(ShowDialog) = SHOW_DIALOG,
    CodeOfConduct(CodeOfConduct) = CODE_OF_CONDUCT,
});

/// ClientInformation.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientInformation {
    pub language: String,
    pub view_distance: i8,
    /// ChatVisiblity: 0 full, 1 system, 2 hidden.
    pub chat_visibility: i32,
    pub chat_colors: bool,
    /// Skin layer bits (PlayerModelPart).
    pub model_customisation: u8,
    /// HumanoidArm: 0 left, 1 right.
    pub main_hand: i32,
    pub text_filtering: bool,
    pub allows_listing: bool,
    /// ParticleStatus: 0 all, 1 decreased, 2 minimal.
    pub particle_status: i32,
}
impl ClientInformation {
    /// ClientInformation.createDefault with a view distance.
    pub fn new(view_distance: i8) -> Self {
        Self {
            language: "en_us".into(),
            view_distance,
            chat_visibility: 0,
            chat_colors: true,
            model_customisation: 0x7F,
            main_hand: 1,
            text_filtering: false,
            allows_listing: true,
            particle_status: 0,
        }
    }
}
impl Codec<ClientInformation> for ClientInformation {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            language: StrN::<16>::read(r)?,
            view_distance: Byte::read(r)?,
            chat_visibility: VarInt::read(r)?,
            chat_colors: Bool::read(r)?,
            model_customisation: UByte::read(r)?,
            main_hand: VarInt::read(r)?,
            text_filtering: Bool::read(r)?,
            allows_listing: Bool::read(r)?,
            particle_status: VarInt::read(r)?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.string(&v.language);
        w.i8(v.view_distance);
        w.varint(v.chat_visibility);
        w.bool(v.chat_colors);
        w.u8(v.model_customisation);
        w.varint(v.main_hand);
        w.bool(v.text_filtering);
        w.bool(v.allows_listing);
        w.varint(v.particle_status);
    }
}
packet!(CookieResponse { key: String => Ident, payload: Option<Vec<u8>> => Opt<crate::buf::Bytes> });
packet!(
    /// ServerboundResourcePackPacket.Action: 0 loaded, 1 declined, 2 failed
    /// download, 3 accepted, 4 downloaded, 5 invalid url, 6 failed reload, 7 discarded.
    ResourcePackResponse { id: u128 => Uuid, action: i32 => VarInt }
);

packet_enum!(Serverbound, crate::ids::configuration::serverbound {
    ClientInformation(ClientInformation) = CLIENT_INFORMATION,
    CookieResponse(CookieResponse) = COOKIE_RESPONSE,
    CustomPayload(CustomPayload) = CUSTOM_PAYLOAD,
    FinishConfiguration(Empty) = FINISH_CONFIGURATION,
    KeepAlive(KeepAlive) = KEEP_ALIVE,
    Pong(Ping) = PONG,
    ResourcePack(ResourcePackResponse) = RESOURCE_PACK,
    SelectKnownPacks(SelectKnownPacks) = SELECT_KNOWN_PACKS,
    AcceptCodeOfConduct(Empty) = ACCEPT_CODE_OF_CONDUCT,
    CustomClickAction(CustomClickAction) = CUSTOM_CLICK_ACTION,
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::round_trip;

    #[test]
    fn configuration_round_trips() {
        let biome = Tag::compound([
            ("has_precipitation", Tag::Byte(1)),
            ("temperature", Tag::Float(0.8f32.to_bits())),
        ]);
        for p in [
            Clientbound::CustomPayload(CustomPayload {
                channel: "minecraft:brand".into(),
                data: vec![7, b'v', b'a', b'n', b'i', b'l', b'l', b'a'],
            }),
            Clientbound::Disconnect(Disconnect {
                reason: crate::text::literal("bye"),
            }),
            Clientbound::FinishConfiguration(Empty),
            Clientbound::KeepAlive(KeepAlive { id: 99 }),
            Clientbound::Ping(Ping { id: -3 }),
            Clientbound::RegistryData(RegistryData {
                registry: "minecraft:worldgen/biome".into(),
                entries: vec![
                    RegistryEntry {
                        id: "minecraft:plains".into(),
                        data: Some(biome),
                    },
                    RegistryEntry {
                        id: "minecraft:desert".into(),
                        data: None,
                    },
                ],
            }),
            Clientbound::ResourcePackPush(ResourcePackPush {
                id: 5,
                url: "http://x".into(),
                hash: String::new(),
                required: false,
                prompt: None,
            }),
            Clientbound::UpdateEnabledFeatures(UpdateEnabledFeatures {
                features: vec!["minecraft:vanilla".into()],
            }),
            Clientbound::UpdateTags(UpdateTags {
                registries: vec![RegistryTags {
                    registry: "minecraft:block".into(),
                    tags: vec![TagEntry {
                        name: "minecraft:logs".into(),
                        ids: vec![1, 2, 300],
                    }],
                }],
            }),
            Clientbound::SelectKnownPacks(SelectKnownPacks {
                packs: vec![KnownPack {
                    namespace: "minecraft".into(),
                    id: "core".into(),
                    version: "26.3".into(),
                }],
            }),
            Clientbound::CodeOfConduct(CodeOfConduct {
                text: "Be nice".into(),
            }),
            Clientbound::ServerLinks(ServerLinks {
                links: vec![
                    ServerLink {
                        kind: ServerLinkKind::Known(0),
                        link: "https://bugs.example.net".into(),
                    },
                    ServerLink {
                        kind: ServerLinkKind::Custom(crate::text::literal("Custom link")),
                        link: "https://example.net".into(),
                    },
                ],
            }),
            Clientbound::ShowDialog(ShowDialog {
                dialog: Tag::compound([("type", Tag::String("minecraft:notice".into()))]),
            }),
            Clientbound::ClearDialog(Empty),
        ] {
            round_trip(p, Clientbound::encode, Clientbound::decode);
        }
        for p in [
            Serverbound::ClientInformation(ClientInformation::new(12)),
            Serverbound::FinishConfiguration(Empty),
            Serverbound::KeepAlive(KeepAlive { id: 99 }),
            Serverbound::Pong(Ping { id: -3 }),
            Serverbound::ResourcePack(ResourcePackResponse { id: 5, action: 1 }),
            Serverbound::SelectKnownPacks(SelectKnownPacks { packs: vec![] }),
            Serverbound::AcceptCodeOfConduct(Empty),
            Serverbound::CustomClickAction(CustomClickAction {
                id: "authme:prejoin-register/submit".into(),
                payload: Some(Tag::compound([("password", Tag::String("hunter2".into()))])),
            }),
            Serverbound::CustomClickAction(CustomClickAction {
                id: "minecraft:x".into(),
                payload: None,
            }),
        ] {
            round_trip(p, Serverbound::encode, Serverbound::decode);
        }
    }

    #[test]
    fn client_information_carries_the_main_hand() {
        // Skin Customization's Main Hand: HumanoidArm LEFT is 0, RIGHT is 1.
        for main_hand in [0, 1] {
            round_trip(
                Serverbound::ClientInformation(ClientInformation {
                    main_hand,
                    ..ClientInformation::new(12)
                }),
                Serverbound::encode,
                Serverbound::decode,
            );
        }
    }

    /// The bytes vanilla writes: the length, then the tag (a lone `End` byte
    /// for none).
    #[test]
    fn click_payload_is_length_prefixed() {
        let mut w = Writer::new();
        ClickPayload::write(&None, &mut w);
        assert_eq!(w.data, [1, 0]);
        let mut w = Writer::new();
        ClickPayload::write(&Some(Tag::Byte(1)), &mut w);
        assert_eq!(w.data, [2, 1, 1]);
    }
}
