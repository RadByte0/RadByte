//! Depth-limited Network NBT encoder and decoder.
//!
//! Modern Minecraft versions (1.20.2+) use **Network NBT** (root tag without a root name)
//! for registry data, dimension types, and chat components. Enforces [`MAX_NBT_DEPTH`]
//! to prevent stack-overflow attacks from maliciously nested payloads.

use std::collections::BTreeMap;

use bytes::{Buf, BufMut};

use crate::error::{Error, Result};

/// Maximum allowed nesting depth for NBT compounds and lists.
pub const MAX_NBT_DEPTH: usize = 512;

/// A Named Binary Tag (NBT) value.
#[derive(Debug, Clone, PartialEq)]
pub enum NbtTag {
    /// `TAG_Byte` (1).
    Byte(i8),
    /// `TAG_Short` (2).
    Short(i16),
    /// `TAG_Int` (3).
    Int(i32),
    /// `TAG_Long` (4).
    Long(i64),
    /// `TAG_Float` (5).
    Float(f32),
    /// `TAG_Double` (6).
    Double(f64),
    /// `TAG_Byte_Array` (7).
    ByteArray(Vec<i8>),
    /// `TAG_String` (8).
    String(String),
    /// `TAG_List` (9).
    List(Vec<NbtTag>),
    /// `TAG_Compound` (10).
    Compound(BTreeMap<String, NbtTag>),
    /// `TAG_Int_Array` (11).
    IntArray(Vec<i32>),
    /// `TAG_Long_Array` (12).
    LongArray(Vec<i64>),
}

impl NbtTag {
    /// Returns the numeric NBT tag type ID.
    #[must_use]
    pub const fn tag_id(&self) -> u8 {
        match self {
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }
}

/// Decodes a Network NBT root tag (type ID followed directly by payload, no root name).
pub fn decode_network_nbt(buf: &mut impl Buf) -> Result<NbtTag> {
    if !buf.has_remaining() {
        return Err(Error::UnexpectedEof { needed: 1 });
    }
    let tag_id = buf.get_u8();
    if tag_id == 0 {
        return Err(Error::Nbt("unexpected TAG_End at root".into()));
    }
    decode_payload(buf, tag_id, 0)
}

/// Encodes a Network NBT root tag (type ID followed directly by payload, no root name).
pub fn encode_network_nbt(tag: &NbtTag, buf: &mut impl BufMut) -> Result<()> {
    buf.put_u8(tag.tag_id());
    encode_payload(tag, buf, 0)
}

/// Decodes a standard named NBT root tag (type ID + 2-byte root name + payload), as used in `.mca` and `level.dat`.
pub fn decode_named_nbt(buf: &mut impl Buf) -> Result<(String, NbtTag)> {
    if !buf.has_remaining() {
        return Err(Error::UnexpectedEof { needed: 1 });
    }
    let tag_id = buf.get_u8();
    if tag_id == 0 {
        return Err(Error::Nbt("unexpected TAG_End at root".into()));
    }
    let name = read_nbt_string(buf)?;
    let tag = decode_payload(buf, tag_id, 0)?;
    Ok((name, tag))
}

/// Encodes a standard named NBT root tag (type ID + 2-byte root name + payload).
pub fn encode_named_nbt(name: &str, tag: &NbtTag, buf: &mut impl BufMut) -> Result<()> {
    buf.put_u8(tag.tag_id());
    write_nbt_string(name, buf)?;
    encode_payload(tag, buf, 0)
}

fn read_nbt_string(buf: &mut impl Buf) -> Result<String> {
    if buf.remaining() < 2 {
        return Err(Error::UnexpectedEof {
            needed: 2 - buf.remaining(),
        });
    }
    let len = usize::from(buf.get_u16());
    if buf.remaining() < len {
        return Err(Error::UnexpectedEof {
            needed: len - buf.remaining(),
        });
    }
    let mut raw = vec![0u8; len];
    buf.copy_to_slice(&mut raw);
    String::from_utf8(raw).map_err(|e| Error::Nbt(format!("invalid utf8 string: {e}")))
}

fn write_nbt_string(s: &str, buf: &mut impl BufMut) -> Result<()> {
    let len =
        u16::try_from(s.len()).map_err(|_| Error::Nbt("NBT string exceeds 65535 bytes".into()))?;
    buf.put_u16(len);
    buf.put_slice(s.as_bytes());
    Ok(())
}

fn ensure_remaining(buf: &impl Buf, needed: usize) -> Result<()> {
    if buf.remaining() < needed {
        Err(Error::UnexpectedEof {
            needed: needed - buf.remaining(),
        })
    } else {
        Ok(())
    }
}

fn decode_payload(buf: &mut impl Buf, tag_id: u8, depth: usize) -> Result<NbtTag> {
    if depth > MAX_NBT_DEPTH {
        return Err(Error::Nbt(format!(
            "exceeded max NBT depth {MAX_NBT_DEPTH}"
        )));
    }

    match tag_id {
        1 => {
            ensure_remaining(buf, 1)?;
            Ok(NbtTag::Byte(buf.get_i8()))
        }
        2 => {
            ensure_remaining(buf, 2)?;
            Ok(NbtTag::Short(buf.get_i16()))
        }
        3 => {
            ensure_remaining(buf, 4)?;
            Ok(NbtTag::Int(buf.get_i32()))
        }
        4 => {
            ensure_remaining(buf, 8)?;
            Ok(NbtTag::Long(buf.get_i64()))
        }
        5 => {
            ensure_remaining(buf, 4)?;
            Ok(NbtTag::Float(buf.get_f32()))
        }
        6 => {
            ensure_remaining(buf, 8)?;
            Ok(NbtTag::Double(buf.get_f64()))
        }
        7 => {
            ensure_remaining(buf, 4)?;
            let len = buf.get_i32();
            if len < 0 {
                return Err(Error::Nbt(format!("negative byte array len: {len}")));
            }
            let len = len as usize;
            ensure_remaining(buf, len)?;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                out.push(buf.get_i8());
            }
            Ok(NbtTag::ByteArray(out))
        }
        8 => Ok(NbtTag::String(read_nbt_string(buf)?)),
        9 => {
            ensure_remaining(buf, 5)?;
            let elem_id = buf.get_u8();
            let len = buf.get_i32();
            if len < 0 {
                return Err(Error::Nbt(format!("negative list len: {len}")));
            }
            let len = len as usize;
            let mut list = Vec::with_capacity(len.min(1024));
            if elem_id == 0 && len > 0 {
                return Err(Error::Nbt(
                    "non-empty TAG_List with TAG_End elements".into(),
                ));
            }
            for _ in 0..len {
                list.push(decode_payload(buf, elem_id, depth + 1)?);
            }
            Ok(NbtTag::List(list))
        }
        10 => {
            let mut map = BTreeMap::new();
            loop {
                ensure_remaining(buf, 1)?;
                let child_id = buf.get_u8();
                if child_id == 0 {
                    break;
                }
                let name = read_nbt_string(buf)?;
                let value = decode_payload(buf, child_id, depth + 1)?;
                map.insert(name, value);
            }
            Ok(NbtTag::Compound(map))
        }
        11 => {
            ensure_remaining(buf, 4)?;
            let len = buf.get_i32();
            if len < 0 {
                return Err(Error::Nbt(format!("negative int array len: {len}")));
            }
            let len = len as usize;
            ensure_remaining(buf, len.saturating_mul(4))?;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                out.push(buf.get_i32());
            }
            Ok(NbtTag::IntArray(out))
        }
        12 => {
            ensure_remaining(buf, 4)?;
            let len = buf.get_i32();
            if len < 0 {
                return Err(Error::Nbt(format!("negative long array len: {len}")));
            }
            let len = len as usize;
            ensure_remaining(buf, len.saturating_mul(8))?;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                out.push(buf.get_i64());
            }
            Ok(NbtTag::LongArray(out))
        }
        other => Err(Error::Nbt(format!("unknown NBT tag id: {other}"))),
    }
}

fn encode_payload(tag: &NbtTag, buf: &mut impl BufMut, depth: usize) -> Result<()> {
    if depth > MAX_NBT_DEPTH {
        return Err(Error::Nbt(format!(
            "exceeded max NBT depth {MAX_NBT_DEPTH}"
        )));
    }

    match tag {
        NbtTag::Byte(v) => buf.put_i8(*v),
        NbtTag::Short(v) => buf.put_i16(*v),
        NbtTag::Int(v) => buf.put_i32(*v),
        NbtTag::Long(v) => buf.put_i64(*v),
        NbtTag::Float(v) => buf.put_f32(*v),
        NbtTag::Double(v) => buf.put_f64(*v),
        NbtTag::ByteArray(arr) => {
            buf.put_i32(i32::try_from(arr.len()).map_err(|_| Error::Nbt("array too big".into()))?);
            for &v in arr {
                buf.put_i8(v);
            }
        }
        NbtTag::String(s) => write_nbt_string(s, buf)?,
        NbtTag::List(items) => {
            let elem_id = items.first().map_or(0, NbtTag::tag_id);
            buf.put_u8(elem_id);
            buf.put_i32(i32::try_from(items.len()).map_err(|_| Error::Nbt("list too big".into()))?);
            for item in items {
                if item.tag_id() != elem_id {
                    return Err(Error::Nbt("heterogeneous TAG_List is not allowed".into()));
                }
                encode_payload(item, buf, depth + 1)?;
            }
        }
        NbtTag::Compound(map) => {
            for (k, v) in map {
                buf.put_u8(v.tag_id());
                write_nbt_string(k, buf)?;
                encode_payload(v, buf, depth + 1)?;
            }
            buf.put_u8(0); // TAG_End
        }
        NbtTag::IntArray(arr) => {
            buf.put_i32(i32::try_from(arr.len()).map_err(|_| Error::Nbt("array too big".into()))?);
            for &v in arr {
                buf.put_i32(v);
            }
        }
        NbtTag::LongArray(arr) => {
            buf.put_i32(i32::try_from(arr.len()).map_err(|_| Error::Nbt("array too big".into()))?);
            for &v in arr {
                buf.put_i64(v);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bytes::BytesMut;
    use fastnbt::{ByteArray, DeOpts, IntArray, LongArray, SerOpts, Value as FastNbtValue};

    use super::*;

    #[test]
    fn network_nbt_roundtrip() {
        let mut root = BTreeMap::new();
        root.insert("name".into(), NbtTag::String("RadByte".into()));
        root.insert("protocol".into(), NbtTag::Int(777));
        root.insert(
            "tags".into(),
            NbtTag::List(vec![
                NbtTag::String("fast".into()),
                NbtTag::String("safe".into()),
            ]),
        );

        let tag = NbtTag::Compound(root);
        let mut buf = BytesMut::new();
        encode_network_nbt(&tag, &mut buf).unwrap();
        let decoded = decode_network_nbt(&mut buf).unwrap();
        assert_eq!(decoded, tag);
    }

    #[test]
    fn network_nbt_is_compatible_with_fastnbt() {
        let mut external_compound = HashMap::new();
        external_compound.insert("byte".into(), FastNbtValue::Byte(-7));
        external_compound.insert("short".into(), FastNbtValue::Short(32000));
        external_compound.insert("int".into(), FastNbtValue::Int(777));
        external_compound.insert("long".into(), FastNbtValue::Long(1_i64 << 40));
        external_compound.insert("float".into(), FastNbtValue::Float(1.25));
        external_compound.insert("double".into(), FastNbtValue::Double(-2.5));
        external_compound.insert(
            "byte_array".into(),
            FastNbtValue::ByteArray(ByteArray::new(vec![-1, 0, 1])),
        );
        external_compound.insert("string".into(), FastNbtValue::String("RadByte".into()));
        external_compound.insert(
            "list".into(),
            FastNbtValue::List(vec![FastNbtValue::Int(1), FastNbtValue::Int(2)]),
        );
        external_compound.insert(
            "int_array".into(),
            FastNbtValue::IntArray(IntArray::new(vec![-1, 0, 1])),
        );
        external_compound.insert(
            "long_array".into(),
            FastNbtValue::LongArray(LongArray::new(vec![-1, 0, 1])),
        );
        let external = FastNbtValue::Compound(external_compound);

        let fastnbt_wire = fastnbt::to_bytes_with_opts(&external, SerOpts::network_nbt()).unwrap();
        let mut fastnbt_input = BytesMut::from(fastnbt_wire.as_slice());
        let decoded_by_radbyte = decode_network_nbt(&mut fastnbt_input).unwrap();
        assert!(fastnbt_input.is_empty());
        assert_eq!(decoded_by_radbyte, to_radbyte_value(external.clone()));

        let mut radbyte_wire = BytesMut::new();
        encode_network_nbt(&decoded_by_radbyte, &mut radbyte_wire).unwrap();
        let decoded_by_fastnbt: FastNbtValue =
            fastnbt::from_bytes_with_opts(&radbyte_wire, DeOpts::network_nbt().max_seq_len(1024))
                .unwrap();
        assert_eq!(decoded_by_fastnbt, external);
    }

    fn to_radbyte_value(value: FastNbtValue) -> NbtTag {
        match value {
            FastNbtValue::Byte(value) => NbtTag::Byte(value),
            FastNbtValue::Short(value) => NbtTag::Short(value),
            FastNbtValue::Int(value) => NbtTag::Int(value),
            FastNbtValue::Long(value) => NbtTag::Long(value),
            FastNbtValue::Float(value) => NbtTag::Float(value),
            FastNbtValue::Double(value) => NbtTag::Double(value),
            FastNbtValue::String(value) => NbtTag::String(value),
            FastNbtValue::ByteArray(value) => NbtTag::ByteArray(value.into_inner()),
            FastNbtValue::IntArray(value) => NbtTag::IntArray(value.into_inner()),
            FastNbtValue::LongArray(value) => NbtTag::LongArray(value.into_inner()),
            FastNbtValue::List(values) => {
                NbtTag::List(values.into_iter().map(to_radbyte_value).collect())
            }
            FastNbtValue::Compound(values) => NbtTag::Compound(
                values
                    .into_iter()
                    .map(|(name, value)| (name, to_radbyte_value(value)))
                    .collect(),
            ),
        }
    }
}
