use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::digest::{SHA256, digest};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use tracing::{debug, info, warn};

const PBKDF2_ITERATIONS: u32 = 600_000;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12; // AES-256-GCM nonce
const KEY_LEN: usize = 32; // 256 bits
const GENERATED_PASSPHRASE_LEN: usize = 32;
const KEYRING_SERVICE: &str = "opencrust";
const KEYRING_ACCOUNT_PREFIX: &str = "vault-passphrase";

#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultFileFingerprint {
    len: u64,
    modified_unix_nanos: u128,
}

#[derive(Debug, Clone)]
struct VaultGetCacheEntry {
    fingerprint: VaultFileFingerprint,
    passphrase_hash: [u8; 32],
    entries: HashMap<String, String>,
}

static VAULT_GET_CACHE: OnceLock<RwLock<HashMap<PathBuf, VaultGetCacheEntry>>> = OnceLock::new();

fn vault_get_cache() -> &'static RwLock<HashMap<PathBuf, VaultGetCacheEntry>> {
    VAULT_GET_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// On-disk representation of the encrypted vault.
#[derive(Debug, Serialize, Deserialize)]
struct VaultFile {
    salt: String,
    nonce: String,
    ciphertext: String,
}

/// Encrypted key-value credential store backed by AES-256-GCM.
///
/// Credentials are kept in memory as a plain `HashMap` after decryption.
/// Call [`save`] to persist changes back to disk.
pub struct CredentialVault {
    path: PathBuf,
    derived_key: Vec<u8>,
    salt: Vec<u8>,
    entries: HashMap<String, String>,
}

impl CredentialVault {
    /// Check whether a vault file exists at `path`.
    pub fn exists(path: &Path) -> bool {
        path.is_file()
    }

    /// Create a brand-new vault at `path`, encrypted with `passphrase`.
    pub fn create(path: &Path, passphrase: &str) -> Result<Self, CredentialError> {
        if path.exists() {
            return Err(CredentialError::AlreadyExists(path.display().to_string()));
        }

        let rng = SystemRandom::new();
        let mut salt = vec![0u8; SALT_LEN];
        rng.fill(&mut salt)
            .map_err(|_| CredentialError::Crypto("failed to generate salt".into()))?;

        let derived_key = derive_key(passphrase, &salt);

        let vault = Self {
            path: path.to_path_buf(),
            derived_key,
            salt,
            entries: HashMap::new(),
        };
        vault.save()?;

        info!("created new credential vault at {}", path.display());
        Ok(vault)
    }

    /// Open an existing vault, decrypting with `passphrase`.
    pub fn open(path: &Path, passphrase: &str) -> Result<Self, CredentialError> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            CredentialError::Io(format!("failed to read vault at {}: {e}", path.display()))
        })?;

        let vault_file: VaultFile = serde_json::from_str(&contents)
            .map_err(|e| CredentialError::Format(format!("invalid vault format: {e}")))?;

        let salt = BASE64
            .decode(&vault_file.salt)
            .map_err(|e| CredentialError::Format(format!("invalid salt encoding: {e}")))?;
        let nonce_bytes = BASE64
            .decode(&vault_file.nonce)
            .map_err(|e| CredentialError::Format(format!("invalid nonce encoding: {e}")))?;
        let mut ciphertext = BASE64
            .decode(&vault_file.ciphertext)
            .map_err(|e| CredentialError::Format(format!("invalid ciphertext encoding: {e}")))?;

        let derived_key = derive_key(passphrase, &salt);

        // Decrypt in place
        let key = make_aead_key(&derived_key)?;
        let nonce = Nonce::try_assume_unique_for_key(&nonce_bytes)
            .map_err(|_| CredentialError::Crypto("invalid nonce length".into()))?;

        let plaintext = key
            .open_in_place(nonce, Aad::empty(), &mut ciphertext)
            .map_err(|_| CredentialError::WrongPassphrase)?;

        let entries: HashMap<String, String> = serde_json::from_slice(plaintext)
            .map_err(|e| CredentialError::Format(format!("corrupted vault data: {e}")))?;

        debug!(
            "opened credential vault at {} ({} keys)",
            path.display(),
            entries.len()
        );

        Ok(Self {
            path: path.to_path_buf(),
            derived_key,
            salt,
            entries,
        })
    }

    /// Retrieve a credential by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|s| s.as_str())
    }

    /// Store or update a credential.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.entries.insert(key.into(), value.into());
    }

    /// Remove a credential. Returns `true` if it existed.
    pub fn remove(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// List all stored credential keys.
    pub fn list_keys(&self) -> Vec<&str> {
        self.entries.keys().map(|k| k.as_str()).collect()
    }

    /// Encrypt and persist the vault to disk.
    pub fn save(&self) -> Result<(), CredentialError> {
        let plaintext = serde_json::to_vec(&self.entries)
            .map_err(|e| CredentialError::Format(format!("failed to serialize vault: {e}")))?;

        let rng = SystemRandom::new();
        let mut nonce_bytes = vec![0u8; NONCE_LEN];
        rng.fill(&mut nonce_bytes)
            .map_err(|_| CredentialError::Crypto("failed to generate nonce".into()))?;

        let key = make_aead_key(&self.derived_key)?;
        let nonce = Nonce::try_assume_unique_for_key(&nonce_bytes)
            .map_err(|_| CredentialError::Crypto("invalid nonce length".into()))?;

        let mut in_out = plaintext;
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| CredentialError::Crypto("encryption failed".into()))?;

        let vault_file = VaultFile {
            salt: BASE64.encode(&self.salt),
            nonce: BASE64.encode(&nonce_bytes),
            ciphertext: BASE64.encode(&in_out),
        };

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CredentialError::Io(format!(
                    "failed to create vault directory {}: {e}",
                    parent.display()
                ))
            })?;
        }

        let json = serde_json::to_string_pretty(&vault_file)
            .map_err(|e| CredentialError::Format(format!("failed to serialize vault file: {e}")))?;
        std::fs::write(&self.path, json).map_err(|e| {
            CredentialError::Io(format!(
                "failed to write vault at {}: {e}",
                self.path.display()
            ))
        })?;

        Ok(())
    }
}

/// Try to load a credential from the vault, returning `None` if the vault
/// doesn't exist or can't be opened (no passphrase prompt in server mode).
pub fn try_vault_get(vault_path: &Path, key: &str) -> Option<String> {
    if !CredentialVault::exists(vault_path) {
        return None;
    }

    let passphrase = resolve_vault_passphrase(vault_path, false)?;
    let passphrase_hash = hash_passphrase(&passphrase);
    let fingerprint_before_open = vault_file_fingerprint(vault_path);
    if let Some(fingerprint) = fingerprint_before_open.as_ref()
        && let Some(value) = cached_vault_value(vault_path, key, fingerprint, &passphrase_hash)
    {
        return value;
    }

    match CredentialVault::open(vault_path, &passphrase) {
        Ok(vault) => {
            let value = vault.get(key).map(|s| s.to_string());
            let fingerprint = vault_file_fingerprint(vault_path).or(fingerprint_before_open);
            if let Some(fingerprint) = fingerprint {
                cache_vault_entries(
                    vault_path,
                    fingerprint,
                    passphrase_hash,
                    vault.entries.clone(),
                );
            }
            value
        }
        Err(e) => {
            warn!("could not open credential vault: {e}");
            None
        }
    }
}

/// Try to store a credential in the vault, returning `true` on success.
/// Falls back silently if a vault passphrase is not available (environment
/// variable or OS keychain), or the vault cannot be opened/created.
///
/// For new vaults, this will auto-generate a high-entropy passphrase and store
/// it in the OS keychain when no env var is provided.
pub fn try_vault_set(vault_path: &Path, key: &str, value: &str) -> bool {
    let vault_exists = CredentialVault::exists(vault_path);
    let passphrase = match resolve_vault_passphrase(vault_path, !vault_exists) {
        Some(p) => p,
        None => {
            warn!("try_vault_set: no vault passphrase available (env or OS keychain)");
            return false;
        }
    };
    let passphrase_hash = hash_passphrase(&passphrase);

    let mut vault = if vault_exists {
        match CredentialVault::open(vault_path, &passphrase) {
            Ok(v) => v,
            Err(e) => {
                warn!("try_vault_set: could not open vault: {e}");
                return false;
            }
        }
    } else {
        match CredentialVault::create(vault_path, &passphrase) {
            Ok(v) => v,
            Err(e) => {
                warn!("try_vault_set: could not create vault: {e}");
                return false;
            }
        }
    };

    vault.set(key, value);
    match vault.save() {
        Ok(()) => {
            if let Some(fingerprint) = vault_file_fingerprint(vault_path) {
                cache_vault_entries(
                    vault_path,
                    fingerprint,
                    passphrase_hash,
                    vault.entries.clone(),
                );
            } else {
                invalidate_vault_cache(vault_path);
            }
            info!("stored credential '{key}' in vault");
            true
        }
        Err(e) => {
            warn!("try_vault_set: failed to save vault: {e}");
            false
        }
    }
}

/// Return whether a passphrase source is available for this vault path.
///
/// Sources checked:
/// 1. `OPENCRUST_VAULT_PASSPHRASE`
/// 2. OS keychain entry (Credential Manager / Keychain / Secret Service)
pub fn vault_passphrase_available(vault_path: &Path) -> bool {
    vault_env_passphrase().is_some() || read_keyring_passphrase(vault_path).is_some()
}

fn resolve_vault_passphrase(vault_path: &Path, allow_create: bool) -> Option<String> {
    // Existing vault: prefer OS keychain first and avoid mutating keychain
    // from env until credentials have been proven valid.
    if !allow_create {
        if let Some(keyring_passphrase) = read_keyring_passphrase(vault_path) {
            return Some(keyring_passphrase);
        }
        return vault_env_passphrase();
    }

    // Vault creation path: env passphrase can seed both vault and keychain.
    if let Some(env_passphrase) = vault_env_passphrase() {
        if !write_keyring_passphrase(vault_path, &env_passphrase) {
            warn!("could not mirror vault passphrase to OS keychain; continuing with env var");
        }
        return Some(env_passphrase);
    }

    if let Some(keyring_passphrase) = read_keyring_passphrase(vault_path) {
        return Some(keyring_passphrase);
    }

    let generated = generate_passphrase()?;
    if write_keyring_passphrase(vault_path, &generated) {
        info!("generated vault passphrase and stored it in OS keychain");
        Some(generated)
    } else {
        warn!("failed to store generated vault passphrase in OS keychain");
        None
    }
}

fn vault_env_passphrase() -> Option<String> {
    std::env::var("OPENCRUST_VAULT_PASSPHRASE")
        .ok()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
}

fn generate_passphrase() -> Option<String> {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; GENERATED_PASSPHRASE_LEN];
    if rng.fill(&mut bytes).is_err() {
        warn!("failed to generate vault passphrase bytes");
        return None;
    }
    Some(BASE64.encode(bytes))
}

fn keyring_account_for_path(vault_path: &Path) -> String {
    let path_bytes = vault_path.to_string_lossy();
    let hash = digest(&SHA256, path_bytes.as_bytes());
    let mut hex = String::with_capacity(hash.as_ref().len() * 2);
    for byte in hash.as_ref() {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    format!("{KEYRING_ACCOUNT_PREFIX}:{hex}")
}

fn keyring_entry_for_path(vault_path: &Path) -> Option<keyring::Entry> {
    let account = keyring_account_for_path(vault_path);
    match keyring::Entry::new(KEYRING_SERVICE, &account) {
        Ok(entry) => Some(entry),
        Err(err) => {
            warn!("failed to create keyring entry: {err}");
            None
        }
    }
}

fn read_keyring_passphrase(vault_path: &Path) -> Option<String> {
    let entry = keyring_entry_for_path(vault_path)?;
    match entry.get_password() {
        Ok(value) if value.trim().is_empty() => None,
        Ok(value) => Some(value),
        Err(keyring::Error::NoEntry) => None,
        Err(err) => {
            warn!("could not read vault passphrase from OS keychain: {err}");
            None
        }
    }
}

fn write_keyring_passphrase(vault_path: &Path, passphrase: &str) -> bool {
    let Some(entry) = keyring_entry_for_path(vault_path) else {
        return false;
    };

    match entry.set_password(passphrase) {
        Ok(()) => true,
        Err(err) => {
            warn!("could not write vault passphrase to OS keychain: {err}");
            false
        }
    }
}

fn hash_passphrase(passphrase: &str) -> [u8; 32] {
    let digest = digest(&SHA256, passphrase.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

fn vault_file_fingerprint(vault_path: &Path) -> Option<VaultFileFingerprint> {
    let metadata = std::fs::metadata(vault_path).ok()?;
    let modified = metadata.modified().ok()?;
    let modified_unix_nanos = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    Some(VaultFileFingerprint {
        len: metadata.len(),
        modified_unix_nanos,
    })
}

fn cached_vault_value(
    vault_path: &Path,
    key: &str,
    fingerprint: &VaultFileFingerprint,
    passphrase_hash: &[u8; 32],
) -> Option<Option<String>> {
    let cache = vault_get_cache();
    let guard = match cache.read() {
        Ok(g) => g,
        Err(poisoned) => {
            warn!("vault read cache lock poisoned; recovering");
            poisoned.into_inner()
        }
    };

    let entry = guard.get(vault_path)?;
    if &entry.fingerprint != fingerprint || &entry.passphrase_hash != passphrase_hash {
        return None;
    }
    Some(entry.entries.get(key).cloned())
}

fn cache_vault_entries(
    vault_path: &Path,
    fingerprint: VaultFileFingerprint,
    passphrase_hash: [u8; 32],
    entries: HashMap<String, String>,
) {
    let cache = vault_get_cache();
    let mut guard = match cache.write() {
        Ok(g) => g,
        Err(poisoned) => {
            warn!("vault read cache lock poisoned; recovering");
            poisoned.into_inner()
        }
    };
    guard.insert(
        vault_path.to_path_buf(),
        VaultGetCacheEntry {
            fingerprint,
            passphrase_hash,
            entries,
        },
    );
}

fn invalidate_vault_cache(vault_path: &Path) {
    let cache = vault_get_cache();
    let mut guard = match cache.write() {
        Ok(g) => g,
        Err(poisoned) => {
            warn!("vault read cache lock poisoned; recovering");
            poisoned.into_inner()
        }
    };
    guard.remove(vault_path);
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Vec<u8> {
    let iterations = NonZeroU32::new(PBKDF2_ITERATIONS).expect("iterations > 0");
    let mut key = vec![0u8; KEY_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        passphrase.as_bytes(),
        &mut key,
    );
    key
}

fn make_aead_key(derived: &[u8]) -> Result<LessSafeKey, CredentialError> {
    let unbound = UnboundKey::new(&AES_256_GCM, derived)
        .map_err(|_| CredentialError::Crypto("failed to create AES key".into()))?;
    Ok(LessSafeKey::new(unbound))
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("vault already exists: {0}")]
    AlreadyExists(String),
    #[error("wrong passphrase or corrupted vault")]
    WrongPassphrase,
    #[error("cryptographic error: {0}")]
    Crypto(String),
    #[error("vault format error: {0}")]
    Format(String),
    #[error("I/O error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_vault_path(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "opencrust-vault-test-{label}-{}-{nanos}.json",
            std::process::id()
        ))
    }

    #[test]
    fn create_open_round_trip() {
        let path = temp_vault_path("round-trip");
        let passphrase = "test-passphrase-123";

        let mut vault = CredentialVault::create(&path, passphrase).unwrap();
        vault.set("ANTHROPIC_API_KEY", "sk-ant-test");
        vault.set("OPENAI_API_KEY", "sk-openai-test");
        vault.save().unwrap();

        let vault2 = CredentialVault::open(&path, passphrase).unwrap();
        assert_eq!(vault2.get("ANTHROPIC_API_KEY"), Some("sk-ant-test"));
        assert_eq!(vault2.get("OPENAI_API_KEY"), Some("sk-openai-test"));
        assert_eq!(vault2.list_keys().len(), 2);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let path = temp_vault_path("wrong-pass");
        let vault = CredentialVault::create(&path, "correct").unwrap();
        drop(vault);

        let result = CredentialVault::open(&path, "wrong");
        assert!(result.is_err());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn remove_key() {
        let path = temp_vault_path("remove");

        let mut vault = CredentialVault::create(&path, "pass").unwrap();
        vault.set("key1", "val1");
        assert!(vault.remove("key1"));
        assert!(!vault.remove("key1")); // already removed
        assert!(vault.get("key1").is_none());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn try_vault_get_cache_refreshes_after_external_write() {
        let path = temp_vault_path("cache-refresh");
        let passphrase = "cache-passphrase-123";

        // SAFETY: test-only process-local env mutation.
        unsafe { std::env::set_var("OPENCRUST_VAULT_PASSPHRASE", passphrase) };
        assert!(try_vault_set(&path, "key1", "value1"));
        assert_eq!(try_vault_get(&path, "key1"), Some("value1".to_string()));

        let mut vault = CredentialVault::open(&path, passphrase).unwrap();
        vault.set("key2", "value2");
        vault.save().unwrap();

        assert_eq!(try_vault_get(&path, "key2"), Some("value2".to_string()));

        // SAFETY: test-only process-local env mutation.
        unsafe { std::env::remove_var("OPENCRUST_VAULT_PASSPHRASE") };
        let _ = fs::remove_file(&path);
    }
}
