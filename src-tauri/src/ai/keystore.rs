//! API key storage in the Windows Credential Manager. The key is never written to a file, never
//! returned to the UI (only "is a key stored?"), and never logged.

const SERVICE: &str = "DevPilot";
const USER: &str = "ai-api-key";
const MAX_KEY_LEN: usize = 512;

pub trait SecretStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, secret: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

pub struct CredentialManager {
    service: String,
}

impl CredentialManager {
    pub fn new() -> Self {
        Self {
            service: SERVICE.into(),
        }
    }

    #[cfg(test)]
    fn with_service(service: &str) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, USER)
            .map_err(|_| "The Windows Credential Manager is not available.".to_string())
    }
}

impl SecretStore for CredentialManager {
    fn get(&self) -> Result<Option<String>, String> {
        match self.entry()?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("Could not read the stored API key.".into()),
        }
    }

    fn set(&self, secret: &str) -> Result<(), String> {
        self.entry()?
            .set_password(secret)
            .map_err(|_| "Could not store the API key.".to_string())
    }

    fn clear(&self) -> Result<(), String> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Could not remove the stored API key.".into()),
        }
    }
}

/// Trim and validate a pasted key. Keys are single tokens: no whitespace or control characters.
pub fn clean_key(raw: &str) -> Result<String, String> {
    let k = raw.trim();
    if k.is_empty() {
        return Err("The API key is empty.".into());
    }
    if k.len() > MAX_KEY_LEN || k.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That does not look like an API key (unexpected spaces or length).".into());
    }
    Ok(k.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_cleaning() {
        assert_eq!(clean_key("  sk-abc123  ").unwrap(), "sk-abc123");
        for bad in ["", "   ", "two words", "line\nbreak", &"x".repeat(513)] {
            assert!(clean_key(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn credential_manager_round_trip() {
        // Uses a unique service name so it never touches a real DevPilot key.
        let store =
            CredentialManager::with_service(&format!("DevPilot-test-{}", std::process::id()));
        assert_eq!(store.get().unwrap(), None);
        store.set("test-secret-value").unwrap();
        assert_eq!(store.get().unwrap().as_deref(), Some("test-secret-value"));
        store.set("replaced").unwrap();
        assert_eq!(store.get().unwrap().as_deref(), Some("replaced"));
        store.clear().unwrap();
        assert_eq!(store.get().unwrap(), None);
        store.clear().unwrap(); // clearing twice is fine
    }
}
