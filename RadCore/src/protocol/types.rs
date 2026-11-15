//! Fundamental Minecraft protocol primitives and encoding/decoding traits.
//!
//! All serialization operates directly on [`bytes::Buf`] and [`bytes::BufMut`]
//! to support zero-copy reads and minimal-allocation writes.

use bytes::{Buf, BufMut};
use uuid::Uuid;

use crate::error::{Error, Result};

/// Default maximum length (in UTF-16 code units / chars) for standard Minecraft strings.
pub const DEFAULT_MAX_STRING_LEN: usize = 32_767;

/// Trait for types that can be decoded from a [`bytes::Buf`].
pub trait Decode: Sized {
    /// Decodes a value from `buf`, advancing the buffer cursor.
    fn decode(buf: &mut impl Buf) -> Result<Self>;
}

/// Trait for types that can be encoded into a [`bytes::BufMut`].
pub trait Encode {
    /// Encodes `self` into `buf`.
    fn encode(&self, buf: &mut impl BufMut) -> Result<()>;

    /// Returns the exact encoded byte length if known cheaply, or `None`.
    fn encoded_len(&self) -> Option<usize> {
        None
    }
}

/// Variable-length 32-bit signed integer (`VarInt`) as defined by the Minecraft protocol.
///
/// Uses LEB128-style encoding with 7 data bits per byte and the MSB as a continuation flag.
/// Maximum encoded size is [`VarInt::MAX_BYTES`] (5 bytes).
///
/// # Example
/// ```
/// use bytes::BytesMut;
/// use rad_core::types::{Decode, Encode, VarInt};
///
/// let mut buf = BytesMut::new();
/// VarInt(300).encode(&mut buf).unwrap();
/// assert_eq!(buf.as_ref(), &[0xAC, 0x02]);
///
/// let decoded = VarInt::decode(&mut buf).unwrap();
/// assert_eq!(decoded, VarInt(300));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VarInt(pub i32);

impl VarInt {
    /// Maximum number of bytes a `VarInt` can occupy.
    pub const MAX_BYTES: usize = 5;

    /// Returns the exact number of bytes required to encode this `VarInt`.
    #[must_use]
    #[inline]
    pub const fn encoded_size(self) -> usize {
        let bits = 32 - (self.0 as u32).leading_zeros() as usize;
        (bits.saturating_sub(1) / 7) + 1
    }
}

impl From<i32> for VarInt {
    #[inline]
    fn from(value: i32) -> Self {
        Self(value)
    }
}

impl From<VarInt> for i32 {
    #[inline]
    fn from(value: VarInt) -> Self {
        value.0
    }
}

impl Decode for VarInt {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if !buf.has_remaining() {
            return Err(Error::UnexpectedEof { needed: 1 });
        }
        let first = buf.get_u8();
        if first < 0x80 {
            return Ok(Self(i32::from(first)));
        }

        let mut value = u32::from(first & 0x7F);
        let mut position = 7;

        for _ in 1..Self::MAX_BYTES {
            if !buf.has_remaining() {
                return Err(Error::UnexpectedEof { needed: 1 });
            }
            let byte = buf.get_u8();
            value |= u32::from(byte & 0x7F) << position;

            if (byte & 0x80) == 0 {
                return Ok(Self(value as i32));
            }
            position += 7;
        }

        Err(Error::VarIntTooLong)
    }
}

impl Encode for VarInt {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        let mut value = self.0 as u32;
        if value < 0x80 {
            buf.put_u8(value as u8);
            return Ok(());
        }
        let mut tmp = [0u8; Self::MAX_BYTES];
        let mut i = 0;
        while value >= 0x80 {
            tmp[i] = ((value & 0x7F) | 0x80) as u8;
            value >>= 7;
            i += 1;
        }
        tmp[i] = value as u8;
        buf.put_slice(&tmp[..=i]);
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(self.encoded_size())
    }
}

/// Variable-length 64-bit signed integer (`VarLong`) as defined by the Minecraft protocol.
///
/// Maximum encoded size is [`VarLong::MAX_BYTES`] (10 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VarLong(pub i64);

impl VarLong {
    /// Maximum number of bytes a `VarLong` can occupy.
    pub const MAX_BYTES: usize = 10;

    /// Returns the exact number of bytes required to encode this `VarLong`.
    #[must_use]
    pub const fn encoded_size(self) -> usize {
        let mut v = self.0 as u64;
        let mut size = 1;
        while v >= 0x80 {
            v >>= 7;
            size += 1;
        }
        size
    }
}

impl From<i64> for VarLong {
    #[inline]
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<VarLong> for i64 {
    #[inline]
    fn from(value: VarLong) -> Self {
        value.0
    }
}

impl Decode for VarLong {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        let mut value: u64 = 0;
        let mut position: u32 = 0;

        for _ in 0..Self::MAX_BYTES {
            if !buf.has_remaining() {
                return Err(Error::UnexpectedEof { needed: 1 });
            }
            let byte = buf.get_u8();
            value |= u64::from(byte & 0x7F) << position;

            if (byte & 0x80) == 0 {
                return Ok(Self(value as i64));
            }
            position += 7;
        }

        Err(Error::VarLongTooLong)
    }
}

impl Encode for VarLong {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        let mut value = self.0 as u64;
        loop {
            if (value & !0x7F) == 0 {
                buf.put_u8(value as u8);
                return Ok(());
            }
            buf.put_u8(((value & 0x7F) | 0x80) as u8);
            value >>= 7;
        }
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(self.encoded_size())
    }
}

/// Reads a length-prefixed UTF-8 string from `buf`, enforcing `max_chars` (UTF-16 units / chars).
///
/// In the Minecraft protocol, strings are prefixed with a `VarInt` byte length.
pub fn read_string(buf: &mut impl Buf, max_chars: usize) -> Result<String> {
    let byte_len = VarInt::decode(buf)?.0;
    if byte_len < 0 {
        return Err(Error::InvalidString(format!("negative length: {byte_len}")));
    }
    let byte_len = byte_len as usize;
    let max_bytes = max_chars.saturating_mul(3);
    if byte_len > max_bytes {
        return Err(Error::InvalidString(format!(
            "byte length {byte_len} exceeds max {max_bytes}"
        )));
    }
    if buf.remaining() < byte_len {
        return Err(Error::UnexpectedEof {
            needed: byte_len - buf.remaining(),
        });
    }

    let mut raw = vec![0u8; byte_len];
    buf.copy_to_slice(&mut raw);
    let s = String::from_utf8(raw).map_err(|e| Error::InvalidString(e.to_string()))?;
    if s.encode_utf16().count() > max_chars {
        return Err(Error::InvalidString(format!(
            "UTF-16 length exceeds max {max_chars}"
        )));
    }
    Ok(s)
}

/// Writes a length-prefixed UTF-8 string into `buf`, enforcing `max_chars`.
pub fn write_string(buf: &mut impl BufMut, s: &str, max_chars: usize) -> Result<()> {
    if s.encode_utf16().count() > max_chars {
        return Err(Error::InvalidString(format!(
            "UTF-16 length exceeds max {max_chars}"
        )));
    }
    let byte_len = i32::try_from(s.len())
        .map_err(|_| Error::InvalidString("string too large for i32".into()))?;
    VarInt(byte_len).encode(buf)?;
    buf.put_slice(s.as_bytes());
    Ok(())
}

impl Decode for String {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        read_string(buf, DEFAULT_MAX_STRING_LEN)
    }
}

impl Encode for String {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, self, DEFAULT_MAX_STRING_LEN)
    }

    fn encoded_len(&self) -> Option<usize> {
        let len = i32::try_from(self.len()).ok()?;
        Some(VarInt(len).encoded_size() + self.len())
    }
}

impl Encode for str {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, self, DEFAULT_MAX_STRING_LEN)
    }
}

impl Decode for bool {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if !buf.has_remaining() {
            return Err(Error::UnexpectedEof { needed: 1 });
        }
        Ok(buf.get_u8() != 0)
    }
}

impl Encode for bool {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_u8(u8::from(*self));
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(1)
    }
}

impl Decode for u8 {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if !buf.has_remaining() {
            return Err(Error::UnexpectedEof { needed: 1 });
        }
        Ok(buf.get_u8())
    }
}

impl Encode for u8 {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_u8(*self);
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(1)
    }
}

impl Decode for u16 {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 2 {
            return Err(Error::UnexpectedEof {
                needed: 2 - buf.remaining(),
            });
        }
        Ok(buf.get_u16())
    }
}

impl Encode for u16 {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_u16(*self);
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(2)
    }
}

impl Decode for i16 {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 2 {
            return Err(Error::UnexpectedEof {
                needed: 2 - buf.remaining(),
            });
        }
        Ok(buf.get_i16())
    }
}

impl Encode for i16 {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_i16(*self);
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(2)
    }
}

impl Decode for i32 {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 4 {
            return Err(Error::UnexpectedEof {
                needed: 4 - buf.remaining(),
            });
        }
        Ok(buf.get_i32())
    }
}

impl Encode for i32 {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_i32(*self);
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(4)
    }
}

impl Decode for i64 {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 8 {
            return Err(Error::UnexpectedEof {
                needed: 8 - buf.remaining(),
            });
        }
        Ok(buf.get_i64())
    }
}

impl Encode for i64 {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_i64(*self);
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(8)
    }
}

impl Decode for f32 {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 4 {
            return Err(Error::UnexpectedEof {
                needed: 4 - buf.remaining(),
            });
        }
        Ok(buf.get_f32())
    }
}

impl Encode for f32 {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_f32(*self);
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(4)
    }
}

impl Decode for f64 {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 8 {
            return Err(Error::UnexpectedEof {
                needed: 8 - buf.remaining(),
            });
        }
        Ok(buf.get_f64())
    }
}

impl Encode for f64 {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_f64(*self);
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(8)
    }
}

/// Packed 64-bit Minecraft block coordinates (`Position`).
///
/// Bit layout (from MSB to LSB):
/// - `x`: 26 bits (signed, `-33_554_432 ..= 33_554_431`)
/// - `z`: 26 bits (signed, `-33_554_432 ..= 33_554_431`)
/// - `y`: 12 bits (signed, `-2048 ..= 2047`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockPos {
    /// X coordinate.
    pub x: i32,
    /// Y coordinate.
    pub y: i32,
    /// Z coordinate.
    pub z: i32,
}

impl BlockPos {
    /// Creates a new [`BlockPos`].
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

impl Decode for BlockPos {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        let val = i64::decode(buf)?;
        let x = (val >> 38) as i32;
        let y = ((val << 52) >> 52) as i32;
        let z = ((val << 26) >> 38) as i32;
        Ok(Self { x, y, z })
    }
}

impl Encode for BlockPos {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        let x = i64::from(self.x) & 0x03FF_FFFF;
        let z = i64::from(self.z) & 0x03FF_FFFF;
        let y = i64::from(self.y) & 0x0FFF;
        let packed = (x << 38) | (z << 12) | y;
        packed.encode(buf)
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(8)
    }
}

impl Decode for Uuid {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if buf.remaining() < 16 {
            return Err(Error::UnexpectedEof {
                needed: 16 - buf.remaining(),
            });
        }
        let mut bytes = [0u8; 16];
        buf.copy_to_slice(&mut bytes);
        Ok(Uuid::from_bytes(bytes))
    }
}

impl Encode for Uuid {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        buf.put_slice(self.as_bytes());
        Ok(())
    }

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(16)
    }
}

impl<T: Decode> Decode for Option<T> {
    #[inline]
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        if bool::decode(buf)? {
            Ok(Some(T::decode(buf)?))
        } else {
            Ok(None)
        }
    }
}

impl<T: Encode> Encode for Option<T> {
    #[inline]
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        match self {
            Some(val) => {
                true.encode(buf)?;
                val.encode(buf)
            }
            None => false.encode(buf),
        }
    }
}

/// Low-precision quantized 3D vector (`net.minecraft.network.LpVec3`, MC 1.21.9+ / 26.3).
///
/// Encodes `(0, 0, 0)` as a single `0x00` byte, and non-zero vectors as 6 bytes
/// (plus an optional [`VarInt`] scale continuation when `max(|x|, |y|, |z|) > 3`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LpVec3 {
    /// X component in blocks/tick.
    pub x: f64,
    /// Y component in blocks/tick.
    pub y: f64,
    /// Z component in blocks/tick.
    pub z: f64,
}

impl LpVec3 {
    /// Zero vector.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    const MAX_QUANTIZED: f64 = 32_766.0;
    const MIN_MAGNITUDE: f64 = 3.051_944_088_384_301e-5;

    /// Creates a new [`LpVec3`].
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    #[inline]
    fn sanitize(v: f64) -> f64 {
        if v.is_finite() { v } else { 0.0 }
    }

    #[inline]
    fn pack_axis(normalized: f64) -> u64 {
        ((normalized * 0.5 + 0.5) * Self::MAX_QUANTIZED)
            .round()
            .clamp(0.0, Self::MAX_QUANTIZED) as u64
    }

    #[inline]
    fn unpack_axis(quantized: u64) -> f64 {
        (quantized.min(32_766) as f64 / Self::MAX_QUANTIZED) * 2.0 - 1.0
    }
}

impl Decode for LpVec3 {
    fn decode(buf: &mut impl Buf) -> Result<Self> {
        let b0 = u8::decode(buf)?;
        if b0 == 0 {
            return Ok(Self::ZERO);
        }
        let b1 = u8::decode(buf)?;
        if buf.remaining() < 4 {
            return Err(Error::UnexpectedEof {
                needed: 4 - buf.remaining(),
            });
        }
        let b2_5 = buf.get_u32();
        let packed = (u64::from(b2_5) << 16) | (u64::from(b1) << 8) | u64::from(b0);
        let mut scale = packed & 3;
        if (packed & 4) != 0 {
            let cont = u64::try_from(VarInt::decode(buf)?.0).unwrap_or(0);
            scale |= cont << 2;
        }
        let scale_f = scale as f64;
        let x = Self::unpack_axis((packed >> 3) & 0x7FFF) * scale_f;
        let y = Self::unpack_axis((packed >> 18) & 0x7FFF) * scale_f;
        let z = Self::unpack_axis((packed >> 33) & 0x7FFF) * scale_f;
        Ok(Self { x, y, z })
    }
}

impl Encode for LpVec3 {
    fn encode(&self, buf: &mut impl BufMut) -> Result<()> {
        let x = Self::sanitize(self.x);
        let y = Self::sanitize(self.y);
        let z = Self::sanitize(self.z);
        let max_abs = x.abs().max(y.abs()).max(z.abs());
        if max_abs < Self::MIN_MAGNITUDE {
            buf.put_u8(0);
            return Ok(());
        }
        let scale = max_abs.ceil().clamp(1.0, 1.717_986_918_3e10) as u64;
        let has_cont = scale > 3;
        let low = (scale & 3) | if has_cont { 4 } else { 0 };
        let scale_f = scale as f64;
        let qx = Self::pack_axis(x / scale_f);
        let qy = Self::pack_axis(y / scale_f);
        let qz = Self::pack_axis(z / scale_f);
        let packed = (qz << 33) | (qy << 18) | (qx << 3) | low;
        buf.put_u8((packed & 0xFF) as u8);
        buf.put_u8(((packed >> 8) & 0xFF) as u8);
        buf.put_u32(((packed >> 16) & 0xFFFF_FFFF) as u32);
        if has_cont {
            VarInt((scale >> 2) as i32).encode(buf)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;

    #[test]
    fn varint_roundtrip() {
        let cases = [
            0,
            1,
            127,
            128,
            255,
            300,
            25565,
            2_097_151,
            2_147_483_647,
            -1,
            -2_147_483_648,
        ];

        for &n in &cases {
            let mut buf = BytesMut::new();
            let v = VarInt(n);
            v.encode(&mut buf).unwrap();
            assert_eq!(buf.len(), v.encoded_size());
            let decoded = VarInt::decode(&mut buf).unwrap();
            assert_eq!(decoded, v);
            assert!(!buf.has_remaining());
        }
    }

    #[test]
    fn varint_too_long_errors() {
        let mut buf = BytesMut::from(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01][..]);
        assert!(matches!(
            VarInt::decode(&mut buf),
            Err(Error::VarIntTooLong)
        ));
    }

    #[test]
    fn varlong_roundtrip() {
        let cases = [0_i64, 1, 128, 1_000_000_000_000, i64::MAX, -1, i64::MIN];
        for &n in &cases {
            let mut buf = BytesMut::new();
            let v = VarLong(n);
            v.encode(&mut buf).unwrap();
            assert_eq!(buf.len(), v.encoded_size());
            let decoded = VarLong::decode(&mut buf).unwrap();
            assert_eq!(decoded, v);
        }
    }

    #[test]
    fn string_roundtrip() {
        let mut buf = BytesMut::new();
        let original = "Héllo, RadByte!".to_string();
        original.encode(&mut buf).unwrap();
        let decoded = String::decode(&mut buf).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn block_pos_roundtrip() {
        let cases = [
            BlockPos::new(0, 64, 0),
            BlockPos::new(-12345, -64, 98765),
            BlockPos::new(33_554_431, 2047, -33_554_432),
            BlockPos::new(-33_554_432, -2048, 33_554_431),
        ];
        for pos in cases {
            let mut buf = BytesMut::new();
            pos.encode(&mut buf).unwrap();
            assert_eq!(buf.len(), 8);
            let decoded = BlockPos::decode(&mut buf).unwrap();
            assert_eq!(decoded, pos);
        }
    }

    #[test]
    fn lp_vec3_roundtrip() {
        let mut buf = BytesMut::new();
        LpVec3::ZERO.encode(&mut buf).unwrap();
        assert_eq!(buf.as_ref(), &[0x00]);
        assert_eq!(LpVec3::decode(&mut buf).unwrap(), LpVec3::ZERO);

        let v = LpVec3::new(0.25, -1.5, 4.75);
        v.encode(&mut buf).unwrap();
        let dec = LpVec3::decode(&mut buf).unwrap();
        assert!((dec.x - v.x).abs() < 1e-3);
        assert!((dec.y - v.y).abs() < 1e-3);
        assert!((dec.z - v.z).abs() < 1e-3);
    }

    #[test]
    fn lp_vec3_independent_vectors() {
        // 1. Zero vector encodes to a single 0x00 byte.
        let mut buf = BytesMut::new();
        LpVec3::new(1e-6, -1e-6, 0.0).encode(&mut buf).unwrap();
        assert_eq!(buf.as_ref(), &[0x00]);

        // 2. Unit vector (1.0, 0.0, -1.0) -> scale=1, no continuation bit (0x04 clear).
        buf.clear();
        LpVec3::new(1.0, 0.0, -1.0).encode(&mut buf).unwrap();
        assert_eq!(buf.len(), 6);
        assert_eq!(buf[0] & 0x07, 0x01); // scale=1, has_cont=0
        let dec = LpVec3::decode(&mut buf).unwrap();
        assert!((dec.x - 1.0).abs() < 1e-4);
        assert!(dec.y.abs() < 1e-4);
        assert!((dec.z + 1.0).abs() < 1e-4);

        // 3. Large vector with scale=5 (>3) -> continuation bit 0x04 set + VarInt(5 >> 2 = 1).
        buf.clear();
        LpVec3::new(5.0, -2.5, 0.0).encode(&mut buf).unwrap();
        assert_eq!(buf.len(), 7);
        assert_eq!(buf[0] & 0x07, 0x05); // (5 & 3) | 4 = 5
        assert_eq!(buf[6], 0x01); // VarInt(5 >> 2)
        let dec = LpVec3::decode(&mut buf).unwrap();
        assert!((dec.x - 5.0).abs() < 1e-3);
        assert!((dec.y + 2.5).abs() < 1e-3);
        assert!(dec.z.abs() < 1e-3);
    }
}
