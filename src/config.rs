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

    /// Re-authenticate automatically when the portal expires the session,
    /// instead of failing with exit 3. Defaults to on.
    ///
    /// This portal expires sessions within a day, and the password needed to
    /// mint a new one is already in the keychain — so the default spares the
    /// user a manual `auth login` most days. Set it to `false` to require an
    /// explicit login, which is the right choice if you'd rather the password
    /// only leave the keychain when you say so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_login: Option<bool>,
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

    /// Whether to re-authenticate automatically on session expiry.
    pub fn auto_login(&self) -> bool {
        self.auto_login.unwrap_or(true)
    }
}

/// Config keys settable via `wabhoa config set <key> <value>`.
pub const KNOWN_KEYS: &[&str] = &["base_url", "username", "auto_login"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_login_defaults_on_but_is_overridable() {
        assert!(Config::default().auto_login());
        let off = Config {
            auto_login: Some(false),
            ..Default::default()
        };
        assert!(!off.auto_login());
    }

    #[test]
    fn base_url_falls_back_to_the_default_portal() {
        assert_eq!(Config::default().base_url(), DEFAULT_BASE_URL);
        let other = Config {
            base_url: Some("https://pay.allianceassociationbank.com".into()),
            ..Default::default()
        };
        assert_eq!(other.base_url(), "https://pay.allianceassociationbank.com");
    }
}
