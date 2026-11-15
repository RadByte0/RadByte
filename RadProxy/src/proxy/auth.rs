//! RSA keypair management and Mojang session server verification (`online-mode`).

use rad_core::error::{Error, Result};
use rad_core::packet::login::GameProfileProperty;
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use serde::Deserialize;
use uuid::Uuid;

/// 1024-bit RSA keypair used for `EncryptionRequest` / `EncryptionResponse`.
pub struct ProxyKeypair {
    private_key: RsaPrivateKey,
    /// DER-encoded SubjectPublicKeyInfo bytes sent to clients.
    pub public_key_der: Vec<u8>,
}

impl ProxyKeypair {
    /// Generates a fresh 1024-bit RSA keypair (standard size for Minecraft protocol).
    pub fn generate() -> Result<Self> {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 1024)
            .map_err(|e| Error::Crypto(format!("RSA keygen failed: {e}")))?;
        let public_key_der = private_key
            .to_public_key()
            .to_public_key_der()
            .map_err(|e| Error::Crypto(format!("SPKI encode failed: {e}")))?
            .as_bytes()
            .to_vec();

        Ok(Self {
            private_key,
            public_key_der,
        })
    }

    /// Decrypts a PKCS#1 v1.5 RSA-encrypted payload from `EncryptionResponse`.
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        self.private_key
            .decrypt(Pkcs1v15Encrypt, ciphertext)
            .map_err(|e| Error::Crypto(format!("RSA decrypt failed: {e}")))
    }
}

/// Verified Mojang player profile returned by `hasJoined`.
#[derive(Debug, Clone)]
pub struct VerifiedProfile {
    /// Authenticated Mojang UUID.
    pub uuid: Uuid,
    /// Canonical Mojang username.
    pub name: String,
    /// Signed profile properties (e.g. skin textures).
    pub properties: Vec<GameProfileProperty>,
}

#[derive(Deserialize)]
struct MojangHasJoinedResponse {
    id: String,
    name: String,
    #[serde(default)]
    properties: Vec<MojangProperty>,
}

#[derive(Deserialize)]
struct MojangProperty {
    name: String,
    value: String,
    signature: Option<String>,
}

/// Verifies a player's session with Mojang's `sessionserver.mojang.com`.
pub async fn verify_mojang_session(
    http: &reqwest::Client,
    username: &str,
    server_hash: &str,
) -> Result<VerifiedProfile> {
    let url = format!(
        "https://sessionserver.mojang.com/session/minecraft/hasJoined?username={username}&serverId={server_hash}"
    );
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::Crypto(format!("Mojang session request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(Error::Crypto(format!(
            "Mojang session rejected ({})",
            resp.status()
        )));
    }

    let body: MojangHasJoinedResponse = resp
        .json()
        .await
        .map_err(|e| Error::Crypto(format!("invalid Mojang session JSON: {e}")))?;

    let uuid = Uuid::parse_str(&body.id)
        .map_err(|e| Error::Crypto(format!("invalid Mojang UUID: {e}")))?;

    let properties = body
        .properties
        .into_iter()
        .map(|p| GameProfileProperty {
            name: p.name,
            value: p.value,
            signature: p.signature,
        })
        .collect();

    Ok(VerifiedProfile {
        uuid,
        name: body.name,
        properties,
    })
}
