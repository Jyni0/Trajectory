//! Secure secret storage. Passwords and OAuth refresh tokens never touch the
//! SQLite database — they live in the OS keychain (macOS Keychain, Windows
//! Credential Manager, Secret Service on Linux).

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{AppError, AppResult};

pub const KEY_PASSWORD: &str = "password";
pub const KEY_REFRESH_TOKEN: &str = "refresh_token";

pub trait SecretStore: Send + Sync {
    fn set(&self, account_id: &str, key: &str, value: &str) -> AppResult<()>;
    fn get(&self, account_id: &str, key: &str) -> AppResult<Option<String>>;
    fn delete(&self, account_id: &str, key: &str) -> AppResult<()>;

    /// Removes every secret that may belong to an account.
    fn delete_all(&self, account_id: &str) -> AppResult<()> {
        self.delete(account_id, KEY_PASSWORD)?;
        self.delete(account_id, KEY_REFRESH_TOKEN)
    }

    fn require(&self, account_id: &str, key: &str) -> AppResult<String> {
        self.get(account_id, key)?
            .ok_or_else(|| AppError::Secret(format!("no {key} stored for account {account_id}")))
    }
}

/// OS keychain backed store.
pub struct KeyringStore {
    service: String,
    /// Service name used before a rename; secrets found there are moved over.
    legacy: Option<String>,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into(), legacy: None }
    }

    pub fn with_legacy(mut self, service: impl Into<String>) -> Self {
        self.legacy = Some(service.into());
        self
    }

    fn entry_in(service: &str, account_id: &str, key: &str) -> AppResult<keyring::Entry> {
        Ok(keyring::Entry::new(service, &format!("{account_id}:{key}"))?)
    }

    fn entry(&self, account_id: &str, key: &str) -> AppResult<keyring::Entry> {
        Self::entry_in(&self.service, account_id, key)
    }

    fn read(entry: &keyring::Entry) -> AppResult<Option<String>> {
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn remove(entry: &keyring::Entry) -> AppResult<()> {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl SecretStore for KeyringStore {
    fn set(&self, account_id: &str, key: &str, value: &str) -> AppResult<()> {
        Ok(self.entry(account_id, key)?.set_password(value)?)
    }

    fn get(&self, account_id: &str, key: &str) -> AppResult<Option<String>> {
        if let Some(v) = Self::read(&self.entry(account_id, key)?)? {
            return Ok(Some(v));
        }
        let Some(legacy) = &self.legacy else { return Ok(None) };
        let old = Self::entry_in(legacy, account_id, key)?;
        let Some(v) = Self::read(&old)? else { return Ok(None) };
        // Move it: the old entry is removed only once the new one is written.
        self.set(account_id, key, &v)?;
        let _ = Self::remove(&old);
        Ok(Some(v))
    }

    fn delete(&self, account_id: &str, key: &str) -> AppResult<()> {
        if let Some(legacy) = &self.legacy {
            Self::remove(&Self::entry_in(legacy, account_id, key)?)?;
        }
        Self::remove(&self.entry(account_id, key)?)
    }
}

/// In-memory store for tests and the headless mode.
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    fn k(account_id: &str, key: &str) -> String {
        format!("{account_id}:{key}")
    }
}

impl SecretStore for MemoryStore {
    fn set(&self, account_id: &str, key: &str, value: &str) -> AppResult<()> {
        self.map.lock().unwrap().insert(Self::k(account_id, key), value.to_string());
        Ok(())
    }

    fn get(&self, account_id: &str, key: &str) -> AppResult<Option<String>> {
        Ok(self.map.lock().unwrap().get(&Self::k(account_id, key)).cloned())
    }

    fn delete(&self, account_id: &str, key: &str) -> AppResult<()> {
        self.map.lock().unwrap().remove(&Self::k(account_id, key));
        Ok(())
    }
}
