//! Encryption for clips tagged "password": XChaCha20-Poly1305 with a key
//! derived from a random 256-bit master key kept in the login Keychain. Sealed
//! data is MAGIC + 24-byte nonce + ciphertext, so it can never be mistaken for
//! zstd (which starts with 28 B5 2F FD) or plain bytes stored with a codec.
//!
//! The master key is only ever created when a clip is sealed; reading never
//! creates one (a new key would orphan clips sealed with a lost one). A failed
//! Keychain read is retried later instead of being remembered.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const MAGIC: &[u8] = b"\0CRV1";
const SERVICE: &str = "cliprust";
const ACCOUNT: &str = "clip-key";
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

pub struct Vault {
    enc: [u8; 32],
    mac: [u8; 32],
    check: String,
}

static VAULT: OnceLock<Vault> = OnceLock::new();
static LAST_FAIL: Mutex<Option<Instant>> = Mutex::new(None);

/// The vault if its key exists (Keychain, or `CLIPRUST_KEY` = 64 hex chars).
pub fn get() -> Option<&'static Vault> {
    load(false)
}

/// The vault, creating and storing a new key if there is none yet.
pub fn get_or_create() -> Option<&'static Vault> {
    load(true)
}

/// Use this master key instead of the Keychain's (tests). No effect once loaded.
pub fn use_key(key: [u8; 32]) {
    let _ = VAULT.set(Vault::new(key));
}

fn load(create: bool) -> Option<&'static Vault> {
    if let Some(v) = VAULT.get() {
        return Some(v);
    }
    let mut last = LAST_FAIL.lock().ok()?;
    if !create && last.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) {
        return None;
    }
    match read_key(create) {
        Ok(Some(key)) => {
            *last = None;
            let _ = VAULT.set(Vault::new(key));
            VAULT.get()
        }
        Ok(None) => None,
        Err(e) => {
            crate::log!("vault: {e}");
            *last = Some(Instant::now());
            None
        }
    }
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

fn read_key(create: bool) -> Result<Option<[u8; 32]>, String> {
    if let Ok(k) = std::env::var("CLIPRUST_KEY") {
        return from_hex(&k).map(Some).ok_or("CLIPRUST_KEY must be 64 hex chars".into());
    }
    use security_framework::passwords::{get_generic_password, set_generic_password};
    match get_generic_password(SERVICE, ACCOUNT) {
        Ok(v) => from_hex(&String::from_utf8_lossy(&v)).map(Some).ok_or("bad key in the Keychain".into()),
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND && !create => Ok(None),
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => {
            let key: [u8; 32] = XChaCha20Poly1305::generate_key(&mut OsRng).into();
            set_generic_password(SERVICE, ACCOUNT, to_hex(&key).as_bytes()).map_err(|e| format!("keychain write: {e}"))?;
            crate::log!("vault: new key stored in the Keychain");
            Ok(Some(key))
        }
        Err(e) => Err(format!("keychain read: {e}")),
    }
}

impl Vault {
    fn new(master: [u8; 32]) -> Vault {
        let check = blake3::derive_key("cliprust 2026-11 key check", &master);
        Vault {
            enc: blake3::derive_key("cliprust 2026-11 clip encryption", &master),
            mac: blake3::derive_key("cliprust 2026-11 secret hash", &master),
            check: to_hex(&check[..8]),
        }
    }

    /// Fingerprint of the key, kept in the database to notice a different key.
    pub fn check(&self) -> &str {
        &self.check
    }

    pub fn seal(&self, plain: &[u8]) -> Vec<u8> {
        let cipher = XChaCha20Poly1305::new((&self.enc).into());
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
        XChaCha20Poly1305::new((&self.enc).into()).decrypt(XNonce::from_slice(nonce), ct).ok()
    }

    /// Stands in for a sealed clip's content hash, so a new copy of the same
    /// secret still de-duplicates without the plain hash being stored.
    pub fn secret_hash(&self, hash: &[u8]) -> Vec<u8> {
        let mut out = b"pw:".to_vec();
        out.extend_from_slice(blake3::keyed_hash(&self.mac, hash).as_bytes());
        out
    }
}

pub fn is_sealed(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}
