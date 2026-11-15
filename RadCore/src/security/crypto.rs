//! Minecraft stream encryption (`AES-128-CFB8`) and offline UUID helpers.

use aes::Aes128;
use aes::cipher::{BlockEncrypt, KeyInit};
use cfb8::Encryptor;
use cfb8::cipher::{BlockEncryptMut, KeyIvInit};
use sha1::{Digest, Sha1};
use uuid::Uuid;

/// Minecraft `AES-128-CFB8` symmetric stream cipher pair.
///
/// In the Minecraft protocol, the 16-byte shared secret is used as both the AES-128 key
/// and the initial 16-byte IV. State is preserved across consecutive `encrypt` / `decrypt` calls.
#[derive(Clone)]
pub struct MinecraftCipher {
    encryptor: Encryptor<Aes128>,
    decrypt_aes: Aes128,
    decrypt_iv: [u8; 16],
}

impl MinecraftCipher {
    /// Creates a new [`MinecraftCipher`] from the 16-byte `shared_secret`.
    #[must_use]
    pub fn new(shared_secret: &[u8; 16]) -> Self {
        Self {
            encryptor: Encryptor::new_from_slices(shared_secret, shared_secret)
                .expect("16-byte AES-128 key and IV are always valid"),
            decrypt_aes: Aes128::new_from_slice(shared_secret)
                .expect("16-byte AES-128 key is always valid"),
            decrypt_iv: *shared_secret,
        }
    }

    /// Encrypts `data` in place, advancing the CFB-8 stream state.
    pub fn encrypt(&mut self, data: &mut [u8]) {
        for byte in data {
            let block = core::slice::from_mut(byte).into();
            self.encryptor.encrypt_block_mut(block);
        }
    }

    /// Decrypts `data` in place, advancing the CFB-8 stream state.
    ///
    /// Uses parallelized 16-block AES-128 keystream generation (`encrypt_blocks`) because
    /// CFB-8 decryption feedback depends only on ciphertext bytes.
    pub fn decrypt(&mut self, data: &mut [u8]) {
        let (chunks, remainder) = data.as_chunks_mut::<16>();
        for chunk in chunks {
            let c = *chunk;

            // Build 16 shift-register inputs from `decrypt_iv` + ciphertext bytes.
            let mut blocks: [aes::Block; 16] = core::array::from_fn(|j| {
                let mut b = [0u8; 16];
                let iv_take = 16 - j;
                b[..iv_take].copy_from_slice(&self.decrypt_iv[j..]);
                if j > 0 {
                    b[iv_take..].copy_from_slice(&c[..j]);
                }
                b.into()
            });

            self.decrypt_aes.encrypt_blocks(&mut blocks);
            for (j, byte) in chunk.iter_mut().enumerate() {
                *byte ^= blocks[j][0];
            }
            self.decrypt_iv = c;
        }

        for byte in remainder {
            let cipher_byte = *byte;
            let mut block = aes::Block::from(self.decrypt_iv);
            self.decrypt_aes.encrypt_block(&mut block);
            *byte = cipher_byte ^ block[0];
            self.decrypt_iv.copy_within(1..16, 0);
            self.decrypt_iv[15] = cipher_byte;
        }
    }
}

/// Computes the standard Minecraft offline-mode UUID (`UUIDv3` of `"OfflinePlayer:<username>"`).
#[must_use]
pub fn offline_player_uuid(username: &str) -> Uuid {
    let input = format!("OfflinePlayer:{username}");
    Uuid::new_v3(&Uuid::NAMESPACE_DNS, input.as_bytes())
}

/// Computes Mojang's signed two's-complement SHA-1 hex digest used for
/// `sessionserver.mojang.com/session/minecraft/hasJoined`.
#[must_use]
pub fn mojang_server_hash(server_id: &str, shared_secret: &[u8], public_key_der: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(server_id.as_bytes());
    hasher.update(shared_secret);
    hasher.update(public_key_der);
    let mut digest: [u8; 20] = hasher.finalize().into();

    let negative = (digest[0] & 0x80) != 0;
    if negative {
        // Two's complement negate 160-bit big-endian integer.
        let mut carry = true;
        for byte in digest.iter_mut().rev() {
            *byte = !*byte;
            if carry {
                let (next, overflow) = byte.overflowing_add(1);
                *byte = next;
                carry = overflow;
            }
        }
    }

    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let trimmed = hex.trim_start_matches('0');
    let body = if trimmed.is_empty() { "0" } else { trimmed };

    if negative {
        format!("-{body}")
    } else {
        body.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_cfb8_roundtrip_streaming() {
        let secret = [0x42_u8; 16];
        let mut server = MinecraftCipher::new(&secret);
        let mut client = MinecraftCipher::new(&secret);

        let mut msg1 = *b"Hello ";
        let mut msg2 = *b"RadByte 26.3!";

        server.encrypt(&mut msg1);
        server.encrypt(&mut msg2);
        assert_ne!(&msg1, b"Hello ");

        client.decrypt(&mut msg1);
        client.decrypt(&mut msg2);
        assert_eq!(&msg1, b"Hello ");
        assert_eq!(&msg2, b"RadByte 26.3!");
    }

    #[test]
    fn aes_cfb8_batched_decrypt_matches_reference() {
        use cfb8::Decryptor;
        use cfb8::cipher::BlockDecryptMut;

        let secret = [0xA5_u8; 16];
        let mut enc = MinecraftCipher::new(&secret);
        let mut dec_fast = MinecraftCipher::new(&secret);
        let mut dec_ref = Decryptor::<Aes128>::new_from_slices(&secret, &secret).unwrap();

        for len in [0_usize, 1, 7, 15, 16, 17, 31, 32, 63, 64, 129] {
            let plain: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(31)).collect();
            let mut cipher = plain.clone();
            enc.encrypt(&mut cipher);

            let mut fast_out = cipher.clone();
            dec_fast.decrypt(&mut fast_out);

            let mut ref_out = cipher;
            for b in &mut ref_out {
                dec_ref.decrypt_block_mut(core::slice::from_mut(b).into());
            }

            assert_eq!(fast_out, plain, "mismatch with plaintext at len={len}");
            assert_eq!(fast_out, ref_out, "mismatch with reference at len={len}");
        }
    }

    #[test]
    fn offline_uuid_is_deterministic() {
        let u1 = offline_player_uuid("Steve");
        let u2 = offline_player_uuid("Steve");
        assert_eq!(u1, u2);
        assert_ne!(u1, offline_player_uuid("Alex"));
    }

    #[test]
    fn mojang_hash_matches_known_vectors() {
        // Known Minecraft protocol test vector for "jeb_"
        let hash = mojang_server_hash("", b"jeb_", b"");
        assert_eq!(hash, "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1");
        let hash_pos = mojang_server_hash("", b"Notch", b"");
        assert_eq!(hash_pos, "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48");
    }
}
