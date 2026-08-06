//! Non-secret settings (`~/.config/wabhoa/config.json`).
//!
//! The portal password and the cached session cookies live in the OS keychain
//! (service `piekstra.wabhoa`), never here.

use serde::{Deserialize, Serialize};

/// The Western Alliance Bank community-association payment portal.
///
/// Alliance Association Bank (`pay.allianceassociationbank.com`) runs the same
/// ServiceStack application under a different brand, so pointing `base_url` at
/// it is expected to work — hence the setting.
pub const DEFAULT_BASE_URL: &str = "https://pay.westernalliancebank.com";

/// Keychain account the portal password is stored under.
pub const KEYCHAIN_ACCOUNT: &str = "password";

/// Keychain account for the cached session cookie bundle. Portal reads
/// authenticate with these cookies alone, so caching them is what lets
/// ordinary commands run without re-sending the password every time.
pub const SESSION_ACCOUNT: &str = "session";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Override the portal base URL (default [`DEFAULT_BASE_URL`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Portal login email (identity label only; secrets stay in the keychain).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

impl Config {
    pub fn base_url(&self) -> String {
        self.base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
    }

    /// Resolve the login email: config, then `$WABHOA_USERNAME`.
    pub fn username(&self) -> Option<String> {
        self.username.clone().or_else(|| {
            std::env::var("WABHOA_USERNAME")
                .ok()
                .filter(|s| !s.is_empty())
        })
    }
}

/// Config keys settable via `wabhoa config set <key> <value>`.
pub const KNOWN_KEYS: &[&str] = &["base_url", "username"];
