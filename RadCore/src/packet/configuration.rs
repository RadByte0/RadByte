//! Configuration state packets (Minecraft Java Edition protocol 777 / 26.3).
//!
//! Separated from version-specific registry and tag data tables: packet codecs
//! only handle wire serialization/deserialization, while tag resolution reads
//! from [`crate::registry_provider::RegistryProvider`].
//!
//! Protocol reference: Minecraft Java Edition Configuration state packet
//! formats (`minecraft.wiki/w/Java_Edition_protocol/Packets#Configuration`).

use std::collections::{BTreeMap, HashMap};

use bytes::{Buf, BufMut};

use crate::error::{Error, Result};
use crate::nbt::{NbtTag, decode_network_nbt, encode_network_nbt};
use crate::packet::{Packet, PacketDirection};
use crate::registry_provider::{RegistryProvider, RegistryProviderError};
use crate::state::ConnectionState;
use crate::types::{Decode, Encode, VarInt, read_string, write_string};

const MAX_IDENTIFIER_LEN: usize = 32767;
const MAX_PLUGIN_DATA_BYTES: usize = 1_048_576;
const MAX_KNOWN_PACKS: usize = 1024;
const MAX_REGISTRY_ENTRIES: usize = 65_536;
const MAX_TAG_REGISTRIES: usize = 1024;
const MAX_TAGS_PER_REGISTRY: usize = 65_536;
const MAX_TAG_MEMBERS: usize = 65_536;

fn read_bounded_count(buf: &mut impl Buf, max: usize) -> Result<usize> {
    let raw = VarInt::decode(buf)?.0;
    if raw < 0 || (raw as usize) > max {
        return Err(Error::InvalidPacketLength { length: raw, max });
    }
    Ok(raw as usize)
}

fn write_varint_len(len: usize, buf: &mut impl BufMut) -> Result<()> {
    let len_i32 = i32::try_from(len).map_err(|_| Error::InvalidPacketLength {
        length: i32::MAX,
        max: i32::MAX as usize,
    })?;
    VarInt(len_i32).encode(buf)
}

/// Clientbound `Plugin Message` packet in Configuration state (`0x01`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigClientboundPluginMessage {
    /// Namespaced plugin channel identifier (e.g. `"minecraft:brand"`).
    pub channel: String,
    /// Raw channel payload bytes.
    pub data: Vec<u8>,
}

impl Packet for ConfigClientboundPluginMessage {
    const ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Configuration;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let channel = read_string(buf, MAX_IDENTIFIER_LEN)?;
        let remaining = buf.remaining();
        if remaining > MAX_PLUGIN_DATA_BYTES {
            return Err(Error::InvalidPacketLength {
                length: i32::try_from(remaining).unwrap_or(i32::MAX),
                max: MAX_PLUGIN_DATA_BYTES,
            });
        }
        let mut data = vec![0u8; remaining];
        buf.copy_to_slice(&mut data);
        Ok(Self { channel, data })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        if self.data.len() > MAX_PLUGIN_DATA_BYTES {
            return Err(Error::InvalidPacketLength {
                length: i32::try_from(self.data.len()).unwrap_or(i32::MAX),
                max: MAX_PLUGIN_DATA_BYTES,
            });
        }
        write_string(buf, &self.channel, MAX_IDENTIFIER_LEN)?;
        buf.put_slice(&self.data);
        Ok(())
    }
}

/// Clientbound `Disconnect` packet in Configuration state (`0x02`).
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigurationDisconnect {
    /// Disconnect reason encoded as a Network NBT text component.
    pub reason: NbtTag,
}

impl Packet for ConfigurationDisconnect {
    const ID: i32 = 0x02;
    const STATE: ConnectionState = ConnectionState::Configuration;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            reason: decode_network_nbt(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        encode_network_nbt(&self.reason, buf)
    }
}

/// Clientbound `Finish Configuration` packet (`0x03`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FinishConfiguration;

impl Packet for FinishConfiguration {
    const ID: i32 = 0x03;
    const STATE: ConnectionState = ConnectionState::Configuration;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(_buf: &mut impl Buf) -> Result<Self> {
        Ok(Self)
    }

    fn encode_body(&self, _buf: &mut impl BufMut) -> Result<()> {
        Ok(())
    }
}

/// Serverbound `Acknowledge Finish Configuration` packet (`0x03`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AcknowledgeFinishConfiguration;

impl Packet for AcknowledgeFinishConfiguration {
    const ID: i32 = 0x03;
    const STATE: ConnectionState = ConnectionState::Configuration;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(_buf: &mut impl Buf) -> Result<Self> {
        Ok(Self)
    }

    fn encode_body(&self, _buf: &mut impl BufMut) -> Result<()> {
        Ok(())
    }
}

/// A resource/data pack descriptor exchanged in `Known Packs` packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownPack {
    /// Pack namespace (e.g. `"minecraft"`).
    pub namespace: String,
    /// Pack identifier (e.g. `"core"`).
    pub id: String,
    /// Pack version string (e.g. `"26.3"`).
    pub version: String,
}

fn decode_known_packs(buf: &mut impl Buf) -> Result<Vec<KnownPack>> {
    let count = read_bounded_count(buf, MAX_KNOWN_PACKS)?;
    let mut packs = Vec::with_capacity(count);
    for _ in 0..count {
        let namespace = read_string(buf, MAX_IDENTIFIER_LEN)?;
        let id = read_string(buf, MAX_IDENTIFIER_LEN)?;
        let version = read_string(buf, MAX_IDENTIFIER_LEN)?;
        packs.push(KnownPack {
            namespace,
            id,
            version,
        });
    }
    Ok(packs)
}

fn encode_known_packs(packs: &[KnownPack], buf: &mut impl BufMut) -> Result<()> {
    if packs.len() > MAX_KNOWN_PACKS {
        return Err(Error::InvalidPacketLength {
            length: i32::try_from(packs.len()).unwrap_or(i32::MAX),
            max: MAX_KNOWN_PACKS,
        });
    }
    write_varint_len(packs.len(), buf)?;
    for pack in packs {
        write_string(buf, &pack.namespace, MAX_IDENTIFIER_LEN)?;
        write_string(buf, &pack.id, MAX_IDENTIFIER_LEN)?;
        write_string(buf, &pack.version, MAX_IDENTIFIER_LEN)?;
    }
    Ok(())
}

/// Clientbound `Select Known Packs` packet (`0x0E`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientboundKnownPacks {
    /// Data/resource packs offered by the server.
    pub packs: Vec<KnownPack>,
}

impl Packet for ClientboundKnownPacks {
    const ID: i32 = 0x0E;
    const STATE: ConnectionState = ConnectionState::Configuration;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            packs: decode_known_packs(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        encode_known_packs(&self.packs, buf)
    }
}

/// Serverbound `Select Known Packs` packet (`0x07`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerboundKnownPacks {
    /// Data/resource packs acknowledged by the client.
    pub packs: Vec<KnownPack>,
}

impl Packet for ServerboundKnownPacks {
    const ID: i32 = 0x07;
    const STATE: ConnectionState = ConnectionState::Configuration;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            packs: decode_known_packs(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        encode_known_packs(&self.packs, buf)
    }
}

/// Single entry in a [`RegistryData`] packet.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryEntry {
    /// Namespaced entry identifier (e.g. `"minecraft:overworld"`).
    pub id: String,
    /// Optional Network NBT compound payload (`None` when covered by a known pack).
    pub data: Option<NbtTag>,
}

/// Clientbound `Registry Data` packet (`0x07`).
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryData {
    /// Namespaced registry identifier (e.g. `"minecraft:dimension_type"`).
    pub registry_id: String,
    /// Ordered entries for this registry.
    pub entries: Vec<RegistryEntry>,
}

impl RegistryData {
    /// Minimal synchronized registries for Minecraft 26.3 (`protocol 777`) when
    /// the client acknowledged `minecraft:core`.
    pub fn minimal_vanilla_26_3() -> Vec<Self> {
        vec![
            Self {
                registry_id: "minecraft:dimension_type".to_owned(),
                entries: vec![
                    RegistryEntry {
                        id: "minecraft:overworld".to_owned(),
                        data: None,
                    },
                    RegistryEntry {
                        id: "minecraft:the_nether".to_owned(),
                        data: None,
                    },
                    RegistryEntry {
                        id: "minecraft:the_end".to_owned(),
                        data: None,
                    },
                ],
            },
            Self {
                registry_id: "minecraft:worldgen/biome".to_owned(),
                entries: vec![RegistryEntry {
                    id: "minecraft:plains".to_owned(),
                    data: None,
                }],
            },
            Self {
                registry_id: "minecraft:chat_type".to_owned(),
                entries: vec![RegistryEntry {
                    id: "minecraft:chat".to_owned(),
                    data: None,
                }],
            },
            Self {
                registry_id: "minecraft:damage_type".to_owned(),
                entries: vec![
                    RegistryEntry {
                        id: "minecraft:in_fire".to_owned(),
                        data: None,
                    },
                    RegistryEntry {
                        id: "minecraft:generic".to_owned(),
                        data: None,
                    },
                    RegistryEntry {
                        id: "minecraft:out_of_world".to_owned(),
                        data: None,
                    },
                ],
            },
        ]
    }

    /// Minimal synchronized registries with inline fallback NBT payloads for
    /// clients that did not acknowledge `minecraft:core`.
    pub fn minimal_fallback_nbt_26_3() -> Vec<Self> {
        let mut regs = Self::minimal_vanilla_26_3();
        for reg in &mut regs {
            for entry in &mut reg.entries {
                entry.data = Some(NbtTag::Compound(BTreeMap::new()));
            }
        }
        regs
    }
}

impl Packet for RegistryData {
    const ID: i32 = 0x07;
    const STATE: ConnectionState = ConnectionState::Configuration;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let registry_id = read_string(buf, MAX_IDENTIFIER_LEN)?;
        let count = read_bounded_count(buf, MAX_REGISTRY_ENTRIES)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let id = read_string(buf, MAX_IDENTIFIER_LEN)?;
            let has_data = bool::decode(buf)?;
            let data = if has_data {
                Some(decode_network_nbt(buf)?)
            } else {
                None
            };
            entries.push(RegistryEntry { id, data });
        }
        Ok(Self {
            registry_id,
            entries,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        if self.entries.len() > MAX_REGISTRY_ENTRIES {
            return Err(Error::InvalidPacketLength {
                length: i32::try_from(self.entries.len()).unwrap_or(i32::MAX),
                max: MAX_REGISTRY_ENTRIES,
            });
        }
        write_string(buf, &self.registry_id, MAX_IDENTIFIER_LEN)?;
        write_varint_len(self.entries.len(), buf)?;
        for entry in &self.entries {
            write_string(buf, &entry.id, MAX_IDENTIFIER_LEN)?;
            match &entry.data {
                Some(nbt) => {
                    true.encode(buf)?;
                    encode_network_nbt(nbt, buf)?;
                }
                None => {
                    false.encode(buf)?;
                }
            }
        }
        Ok(())
    }
}

/// Single named tag and its numeric registry member IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEntry {
    /// Namespaced tag name (e.g. `"minecraft:mineable/pickaxe"`).
    pub name: String,
    /// Numeric registry IDs belonging to this tag.
    pub ids: Vec<VarInt>,
}

/// All tags for a single registry in [`ConfigUpdateTags`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRegistry {
    /// Namespaced registry identifier (e.g. `"minecraft:block"`).
    pub registry: String,
    /// Tags belonging to this registry.
    pub tags: Vec<TagEntry>,
}

/// Clientbound `Update Tags` packet in Configuration state (`0x0D`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigUpdateTags {
    /// Per-registry tag lists.
    pub registries: Vec<TagRegistry>,
}

impl ConfigUpdateTags {
    /// Builds a [`ConfigUpdateTags`] packet from a [`RegistryProvider`] and the
    /// synchronized [`RegistryData`] registries.
    pub fn for_registries_with_provider(
        registries: &[RegistryData],
        provider: &dyn RegistryProvider,
    ) -> std::result::Result<Self, RegistryProviderError> {
        let block_tags_defs = provider.block_tags()?;
        let item_tags_defs = provider.item_tags()?;
        let core_sets = provider.core_registry_tags()?;

        let mut out_registries = Vec::with_capacity(2 + core_sets.len());

        let mut block_tags = Vec::with_capacity(block_tags_defs.len());
        for def in block_tags_defs {
            let mut ids = Vec::with_capacity(def.members.len());
            for member in &def.members {
                let entry = provider.block_by_name(member).ok_or_else(|| {
                    RegistryProviderError::ValidationError(format!(
                        "block tag '{}' references unknown block '{}'",
                        def.name, member
                    ))
                })?;
                ids.push(VarInt(i32::from(entry.block_id)));
            }
            block_tags.push(TagEntry {
                name: def.name,
                ids,
            });
        }
        out_registries.push(TagRegistry {
            registry: "minecraft:block".to_owned(),
            tags: block_tags,
        });

        let mut item_tags = Vec::with_capacity(item_tags_defs.len());
        for def in item_tags_defs {
            let mut ids = Vec::with_capacity(def.members.len());
            for member in &def.members {
                let id = provider.item_by_name(member).ok_or_else(|| {
                    RegistryProviderError::ValidationError(format!(
                        "item tag '{}' references unknown item '{}'",
                        def.name, member
                    ))
                })?;
                ids.push(VarInt(id));
            }
            item_tags.push(TagEntry {
                name: def.name,
                ids,
            });
        }
        out_registries.push(TagRegistry {
            registry: "minecraft:item".to_owned(),
            tags: item_tags,
        });

        let reg_index: HashMap<&str, HashMap<&str, i32>> = registries
            .iter()
            .map(|r| {
                let entries = r
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(idx, e)| (e.id.as_str(), idx as i32))
                    .collect();
                (r.registry_id.as_str(), entries)
            })
            .collect();

        for set in core_sets {
            let Some(entries_map) = reg_index.get(set.registry_id.as_str()) else {
                continue;
            };
            let mut tags = Vec::with_capacity(set.tags.len());
            for def in set.tags {
                let mut ids = Vec::with_capacity(def.members.len());
                for member in &def.members {
                    let Some(&idx) = entries_map.get(member.as_str()) else {
                        return Err(RegistryProviderError::ValidationError(format!(
                            "tag '{}' in registry '{}' references unknown entry '{}'",
                            def.name, set.registry_id, member
                        )));
                    };
                    ids.push(VarInt(idx));
                }
                tags.push(TagEntry {
                    name: def.name,
                    ids,
                });
            }
            out_registries.push(TagRegistry {
                registry: set.registry_id,
                tags,
            });
        }

        Ok(Self {
            registries: out_registries,
        })
    }
}

impl Packet for ConfigUpdateTags {
    const ID: i32 = 0x0D;
    const STATE: ConnectionState = ConnectionState::Configuration;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let reg_count = read_bounded_count(buf, MAX_TAG_REGISTRIES)?;
        let mut registries = Vec::with_capacity(reg_count);
        for _ in 0..reg_count {
            let registry = read_string(buf, MAX_IDENTIFIER_LEN)?;
            let tag_count = read_bounded_count(buf, MAX_TAGS_PER_REGISTRY)?;
            let mut tags = Vec::with_capacity(tag_count);
            for _ in 0..tag_count {
                let name = read_string(buf, MAX_IDENTIFIER_LEN)?;
                let member_count = read_bounded_count(buf, MAX_TAG_MEMBERS)?;
                let mut ids = Vec::with_capacity(member_count);
                for _ in 0..member_count {
                    ids.push(VarInt::decode(buf)?);
                }
                tags.push(TagEntry { name, ids });
            }
            registries.push(TagRegistry { registry, tags });
        }
        Ok(Self { registries })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        if self.registries.len() > MAX_TAG_REGISTRIES {
            return Err(Error::InvalidPacketLength {
                length: i32::try_from(self.registries.len()).unwrap_or(i32::MAX),
                max: MAX_TAG_REGISTRIES,
            });
        }
        write_varint_len(self.registries.len(), buf)?;
        for reg in &self.registries {
            if reg.tags.len() > MAX_TAGS_PER_REGISTRY {
                return Err(Error::InvalidPacketLength {
                    length: i32::try_from(reg.tags.len()).unwrap_or(i32::MAX),
                    max: MAX_TAGS_PER_REGISTRY,
                });
            }
            write_string(buf, &reg.registry, MAX_IDENTIFIER_LEN)?;
            write_varint_len(reg.tags.len(), buf)?;
            for tag in &reg.tags {
                if tag.ids.len() > MAX_TAG_MEMBERS {
                    return Err(Error::InvalidPacketLength {
                        length: i32::try_from(tag.ids.len()).unwrap_or(i32::MAX),
                        max: MAX_TAG_MEMBERS,
                    });
                }
                write_string(buf, &tag.name, MAX_IDENTIFIER_LEN)?;
                write_varint_len(tag.ids.len(), buf)?;
                for id in &tag.ids {
                    id.encode(buf)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;
    use crate::codec::{DEFAULT_MAX_PACKET_SIZE, decode_frame, encode_packet};
    use crate::registry_provider::{
        BlockRegistryEntry, RegistryTagSet, SyntheticFixtureRegistryProvider, TagDefinition,
        VersionMetadata,
    };

    #[test]
    fn roundtrip_all_configuration_packets() {
        let mut buf = BytesMut::new();

        let plugin = ConfigClientboundPluginMessage {
            channel: "minecraft:brand".into(),
            data: b"\x07RadByte".to_vec(),
        };
        encode_packet(&plugin, &mut buf).unwrap();
        let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
            .unwrap()
            .unwrap();
        assert_eq!(frame.id, ConfigClientboundPluginMessage::ID);
        assert_eq!(
            frame.decode_as::<ConfigClientboundPluginMessage>().unwrap(),
            plugin
        );

        let dc = ConfigurationDisconnect {
            reason: NbtTag::String("bye".into()),
        };
        encode_packet(&dc, &mut buf).unwrap();
        let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
            .unwrap()
            .unwrap();
        assert_eq!(frame.decode_as::<ConfigurationDisconnect>().unwrap(), dc);

        encode_packet(&FinishConfiguration, &mut buf).unwrap();
        let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
            .unwrap()
            .unwrap();
        assert_eq!(frame.id, FinishConfiguration::ID);
        frame.decode_as::<FinishConfiguration>().unwrap();

        encode_packet(&AcknowledgeFinishConfiguration, &mut buf).unwrap();
        let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
            .unwrap()
            .unwrap();
        assert_eq!(frame.id, AcknowledgeFinishConfiguration::ID);
        frame.decode_as::<AcknowledgeFinishConfiguration>().unwrap();

        let kp_c = ClientboundKnownPacks {
            packs: vec![KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: "26.3".into(),
            }],
        };
        encode_packet(&kp_c, &mut buf).unwrap();
        let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
            .unwrap()
            .unwrap();
        assert_eq!(frame.decode_as::<ClientboundKnownPacks>().unwrap(), kp_c);

        let kp_s = ServerboundKnownPacks {
            packs: kp_c.packs.clone(),
        };
        encode_packet(&kp_s, &mut buf).unwrap();
        let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
            .unwrap()
            .unwrap();
        assert_eq!(frame.decode_as::<ServerboundKnownPacks>().unwrap(), kp_s);

        let regs = RegistryData::minimal_fallback_nbt_26_3();
        for r in &regs {
            encode_packet(r, &mut buf).unwrap();
            let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
                .unwrap()
                .unwrap();
            assert_eq!(&frame.decode_as::<RegistryData>().unwrap(), r);
        }

        let provider = SyntheticFixtureRegistryProvider::minimal_valid();
        let tags = ConfigUpdateTags::for_registries_with_provider(&regs, &provider).unwrap();
        encode_packet(&tags, &mut buf).unwrap();
        let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
            .unwrap()
            .unwrap();
        assert_eq!(frame.decode_as::<ConfigUpdateTags>().unwrap(), tags);
    }

    #[test]
    fn rejects_negative_or_oversized_counts() {
        let mut buf = BytesMut::new();
        VarInt(-1).encode(&mut buf).unwrap();
        assert!(ClientboundKnownPacks::decode_body(&mut buf.clone()).is_err());

        buf.clear();
        VarInt(i32::try_from(MAX_KNOWN_PACKS + 1).unwrap())
            .encode(&mut buf)
            .unwrap();
        assert!(ClientboundKnownPacks::decode_body(&mut buf).is_err());
    }

    struct UnknownMemberProvider;

    impl RegistryProvider for UnknownMemberProvider {
        fn metadata(&self) -> VersionMetadata {
            VersionMetadata {
                game_version: "0.0-synthetic".into(),
                protocol_version: 0,
                world_data_version: 0,
                source: "synthetic-test".into(),
                is_synthetic_fixture: true,
            }
        }
        fn block_by_name(&self, _name: &str) -> Option<BlockRegistryEntry> {
            None
        }
        fn block_by_state(&self, _state: u16) -> Option<BlockRegistryEntry> {
            None
        }
        fn item_by_name(&self, _name: &str) -> Option<i32> {
            None
        }
        fn item_name_by_id(&self, _id: i32) -> Option<&str> {
            None
        }
        fn entity_by_name(&self, _name: &str) -> Option<i32> {
            None
        }
        fn menu_by_name(&self, _name: &str) -> Option<i32> {
            None
        }
        fn block_tags(&self) -> std::result::Result<Vec<TagDefinition>, RegistryProviderError> {
            Ok(vec![TagDefinition {
                name: "test:broken".into(),
                members: vec!["test:missing_block".into()],
            }])
        }
        fn item_tags(&self) -> std::result::Result<Vec<TagDefinition>, RegistryProviderError> {
            Ok(Vec::new())
        }
        fn core_registry_tags(
            &self,
        ) -> std::result::Result<Vec<RegistryTagSet>, RegistryProviderError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn update_tags_rejects_unknown_tag_member() {
        let regs = RegistryData::minimal_vanilla_26_3();
        let err = ConfigUpdateTags::for_registries_with_provider(&regs, &UnknownMemberProvider)
            .unwrap_err();
        assert!(matches!(err, RegistryProviderError::ValidationError(_)));
    }
}
