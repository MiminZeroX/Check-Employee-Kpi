//! Encryption of secrets stored in the database (camera passwords, TOTP secrets,
//! webhook secrets) with AES-256-GCM. Values are stored as `enc:v1:<base64(nonce|ciphertext)>`;
//! older plain values are still readable so existing installations keep working.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};

const PREFIX: &str = "enc:v1:";

pub struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    /// DATA_KEY: 64 hex characters. Without it a key is derived from JWT_SECRET
    /// (works, but rotating JWT_SECRET would then make stored secrets unreadable).
    pub fn from_env(jwt_secret: &str) -> anyhow::Result<Self> {
        let key: [u8; 32] = match std::env::var("DATA_KEY").ok().filter(|v| !v.is_empty()) {
            Some(hexkey) => {
                let k = hex::decode(hexkey.trim()).map_err(|_| anyhow::anyhow!("DATA_KEY must be 64 hex characters"))?;
                k.try_into().map_err(|_| anyhow::anyhow!("DATA_KEY must be 64 hex characters (32 bytes)"))?
            }
            None => {
                tracing::warn!("DATA_KEY not set: deriving the data encryption key from JWT_SECRET");
                Sha256::digest(format!("smartoffice-data-key:{jwt_secret}").as_bytes()).into()
            }
        };
        Ok(Self { cipher: Aes256Gcm::new(&key.into()) })
    }

    pub fn encrypt(&self, plain: &str) -> String {
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = self.cipher.encrypt(Nonce::from_slice(&nonce), plain.as_bytes()).expect("encrypt");
        let mut buf = nonce.to_vec();
        buf.extend(ct);
        format!("{PREFIX}{}", B64.encode(buf))
    }

    /// Decrypts a stored value; plain (legacy) values are returned unchanged.
    pub fn decrypt(&self, stored: &str) -> anyhow::Result<String> {
        let Some(b64) = stored.strip_prefix(PREFIX) else { return Ok(stored.to_string()) };
        let buf = B64.decode(b64)?;
        if buf.len() < 13 {
            anyhow::bail!("ciphertext too short");
        }
        let (nonce, ct) = buf.split_at(12);
        let plain = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| anyhow::anyhow!("cannot decrypt stored secret (wrong DATA_KEY?)"))?;
        Ok(String::from_utf8(plain)?)
    }
}
