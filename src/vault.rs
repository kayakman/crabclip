//! Encryption for clips tagged "password": XChaCha20-Poly1305 with a random
//! 256-bit key kept in the login Keychain (created on first use). Sealed data
//! is MAGIC + 24-byte nonce + ciphertext, so it can never be mistaken for zstd
//! (which starts with 28 B5 2F FD) or plain bytes stored with a codec.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::sync::OnceLock;

pub const MAGIC: &[u8] = b"\0CRV1";
const SERVICE: &str = "cliprust";
const ACCOUNT: &str = "clip-key";
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

pub struct Vault {
    key: [u8; 32],
}

static VAULT: OnceLock<Option<Vault>> = OnceLock::new();

/// The vault, loaded (or created) on first use. `CLIPRUST_KEY` (64 hex chars)
/// overrides the Keychain, for tests and throwaway databases.
pub fn get() -> Option<&'static Vault> {
    VAULT
        .get_or_init(|| match load() {
            Ok(v) => Some(v),
            Err(e) => {
                crate::log!("vault: {e}");
                None
            }
        })
        .as_ref()
}

/// Use this key instead of the Keychain's (tests). No effect once loaded.
pub fn use_key(key: [u8; 32]) {
    let _ = VAULT.set(Some(Vault { key }));
}

fn from_hex(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, k) in key.iter_mut().enumerate() {
        *k = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn load() -> Result<Vault, String> {
    if let Ok(k) = std::env::var("CLIPRUST_KEY") {
        return from_hex(&k).map(|key| Vault { key }).ok_or("CLIPRUST_KEY must be 64 hex chars".into());
    }
    use security_framework::passwords::{get_generic_password, set_generic_password};
    match get_generic_password(SERVICE, ACCOUNT) {
        Ok(v) => {
            let key = from_hex(&String::from_utf8_lossy(&v)).ok_or("bad key in the Keychain")?;
            Ok(Vault { key })
        }
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => {
            let key: [u8; 32] = XChaCha20Poly1305::generate_key(&mut OsRng).into();
            set_generic_password(SERVICE, ACCOUNT, to_hex(&key).as_bytes()).map_err(|e| format!("keychain write: {e}"))?;
            crate::log!("vault: new key stored in the Keychain");
            Ok(Vault { key })
        }
        Err(e) => Err(format!("keychain read: {e}")),
    }
}

impl Vault {
    pub fn seal(&self, plain: &[u8]) -> Vec<u8> {
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = cipher.encrypt(&nonce, plain).expect("xchacha20poly1305 encrypt");
        let mut out = Vec::with_capacity(MAGIC.len() + 24 + ct.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    pub fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        let rest = sealed.strip_prefix(MAGIC)?;
        if rest.len() < 24 {
            return None;
        }
        let (nonce, ct) = rest.split_at(24);
        XChaCha20Poly1305::new((&self.key).into()).decrypt(XNonce::from_slice(nonce), ct).ok()
    }

    /// Stands in for a sealed clip's content hash, so a new copy of the same
    /// secret still de-duplicates without the plain hash being stored.
    pub fn secret_hash(&self, hash: &[u8]) -> Vec<u8> {
        let mut out = b"pw:".to_vec();
        out.extend_from_slice(blake3::keyed_hash(&self.key, hash).as_bytes());
        out
    }
}

pub fn is_sealed(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}
