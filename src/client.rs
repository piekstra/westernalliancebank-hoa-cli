//! HTTP client for the Western Alliance Bank community-association payment
//! portal.
//!
//! There is no published API. Everything here targets the same endpoints the
//! portal's own front end calls, mapped by watching its XHR traffic. See
//! `docs/api.md`.
//!
//! # Auth model
//!
//! The portal is a ServiceStack (ASP.NET) app. `POST /auth/credentials` takes
//! JSON `{UserName, Password, RememberMe}` and, on success, sets a bundle of
//! cookies: `ss-id`/`ss-pid`/`ss-opt` (the ServiceStack session) and `opayUC`
//! (the portal's own user context). Every later read authenticates with those
//! cookies alone — no bearer token, no CSRF token on reads, no refresh flow.
//!
//! No second factor is involved: a correct password logs straight in. That is
//! the portal's design, not an oversight on this client's part.
//!
//! So `wabhoa auth login` posts the password once and caches the resulting
//! cookie bundle in the OS keychain; ordinary commands replay it.

use std::sync::Arc;
use std::time::Duration;

use pk_cli_core::CliError;
use pk_cli_secrets::{CredentialStore, Secret};
use reqwest::cookie::CookieStore;
use serde_json::{json, Value};

use crate::config::{Config, SESSION_ACCOUNT};

/// A recent desktop Chrome UA. The portal sits behind Cloudflare, which
/// rejects obviously-bot clients.
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Cookies that together constitute a logged-in session. `ss-id` is the
/// ServiceStack session; `opayUC` carries the portal's user context and reads
/// fail without it even when `ss-id` is valid.
const SESSION_COOKIES: &[&str] = &["ss-id", "ss-pid", "ss-opt", "opayUC"];

const LOGIN_PATH: &str = "/auth/credentials";

/// A portal session.
pub struct Portal {
    http: reqwest::blocking::Client,
    jar: Arc<reqwest::cookie::Jar>,
    base: String,
    /// Keychain write-back for a rotated cookie bundle. ServiceStack re-issues
    /// session cookies as it pleases; replaying only the originally-seeded
    /// values would eventually go stale despite the portal having just handed
    /// us live ones.
    sync: Option<SessionSync>,
}

struct SessionSync {
    creds: CredentialStore,
    last: std::cell::RefCell<String>,
}

impl Portal {
    /// A client with an empty cookie jar and no keychain write-back.
    pub fn new(base: impl Into<String>) -> Result<Self, CliError> {
        let base = base.into();
        // Validate early so `url()` and `base_url()` can't fail later.
        base.parse::<reqwest::Url>()
            .map_err(|e| CliError::Usage(format!("invalid base_url {base:?}: {e}")))?;
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let http = reqwest::blocking::Client::builder()
            .user_agent(UA)
            // Total request budget plus an explicit connect budget, so a
            // stalled TLS handshake fails fast instead of hanging the CLI.
            .timeout(Duration::from_secs(45))
            .connect_timeout(Duration::from_secs(15))
            .cookie_provider(jar.clone())
            .build()
            .map_err(|e| CliError::Other(format!("failed to build HTTP client: {e}")))?;
        Ok(Portal {
            http,
            jar,
            base,
            sync: None,
        })
    }

    /// Replay a cached session from the keychain. The session is *not* verified
    /// here; the first read surfaces expiry.
    pub fn from_cached_session(cfg: &Config, creds: &CredentialStore) -> Result<Self, CliError> {
        let session = creds.get(SESSION_ACCOUNT)?.ok_or_else(|| {
            CliError::Auth("no portal session stored — run `wabhoa auth login`".into())
        })?;
        let mut portal = Portal::new(cfg.base_url())?;
        portal.seed_bundle(session.expose());
        portal.sync = Some(SessionSync {
            creds: CredentialStore::new(creds.service()),
            last: std::cell::RefCell::new(session.expose().to_string()),
        });
        Ok(portal)
    }

    /// Persist the cookie bundle if the portal rotated any of it. Best-effort:
    /// a keychain hiccup must not fail a read that already succeeded.
    fn sync_session(&self) {
        let Some(sync) = &self.sync else { return };
        let Some(current) = self.session_bundle() else {
            return;
        };
        let current = current.expose().to_string();
        if *sync.last.borrow() == current {
            return;
        }
        if sync
            .creds
            .set(SESSION_ACCOUNT, &Secret::new(current.clone()))
            .is_ok()
        {
            *sync.last.borrow_mut() = current;
        }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else if path.starts_with('/') {
            format!("{}{}", self.base.trim_end_matches('/'), path)
        } else {
            format!("{}/{}", self.base.trim_end_matches('/'), path)
        }
    }

    fn base_url(&self) -> reqwest::Url {
        self.base
            .parse()
            .expect("base URL validated when the client was built")
    }

    /// Read a cookie value back out of the jar.
    fn cookie(&self, name: &str) -> Option<String> {
        let header = self.jar.cookies(&self.base_url())?;
        let raw = header.to_str().ok()?;
        raw.split("; ")
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
            .map(str::to_string)
    }

    /// The live session cookies, serialized for the keychain as a single
    /// `name=value; name=value` string.
    pub fn session_bundle(&self) -> Option<Secret> {
        let pairs: Vec<String> = SESSION_COOKIES
            .iter()
            .filter_map(|n| self.cookie(n).map(|v| format!("{n}={v}")))
            .collect();
        // `ss-id` alone is what actually authenticates; the rest ride along.
        pairs
            .iter()
            .any(|p| p.starts_with("ss-id="))
            .then(|| Secret::new(pairs.join("; ")))
    }

    /// Seed a serialized cookie bundle back into the jar.
    fn seed_bundle(&self, bundle: &str) {
        for pair in bundle.split(';') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            self.jar
                .add_cookie_str(&format!("{pair}; Path=/; Secure"), &self.base_url());
        }
    }

    // ---- Authentication ----------------------------------------------------

    /// Exchange a username and password for a session.
    pub fn login(&self, username: &str, password: &Secret) -> Result<(), CliError> {
        let body = json!({
            "UserName": username,
            "Password": password.expose(),
            "RememberMe": false,
        });
        let resp = self
            .http
            .post(self.url(LOGIN_PATH))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Origin", self.base.trim_end_matches('/'))
            .json(&body)
            .send()
            .map_err(|e| CliError::Upstream(format!("login request failed: {e}")))?;

        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if status.as_u16() == 401 {
            // ServiceStack answers bad credentials with a typed error body.
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| {
                    v.pointer("/ResponseStatus/Message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "invalid username or password".into());
            return Err(CliError::Auth(format!(
                "{msg} — check `wabhoa config show`, then re-run `wabhoa auth login`"
            )));
        }
        if !status.is_success() {
            return Err(CliError::Upstream(format!(
                "login returned HTTP {}{}",
                status.as_u16(),
                body_hint(&text)
            )));
        }
        Ok(())
    }

    // ---- Reads -------------------------------------------------------------

    /// GET a portal path, returning the response body as text (HTML or JSON).
    pub fn get_text(&self, path: &str) -> Result<String, CliError> {
        let resp = self
            .http
            .get(self.url(path))
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| CliError::Upstream(format!("GET {path} failed: {e}")))?;
        let out = self.handle_text(resp, path);
        if out.is_ok() {
            self.sync_session();
        }
        out
    }

    /// POST a JSON body to a portal path expecting a JSON reply.
    ///
    /// Read-only by construction: every caller in this crate posts to a query
    /// endpoint. The portal uses POST for searches because its search filters
    /// exceed what it cares to put in a query string, not because they mutate.
    pub fn post_json(&self, path: &str, body: &Value) -> Result<Value, CliError> {
        let resp = self
            .http
            .post(self.url(path))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Origin", self.base.trim_end_matches('/'))
            .json(body)
            .send()
            .map_err(|e| CliError::Upstream(format!("POST {path} failed: {e}")))?;
        let text = self.handle_text(resp, path)?;
        self.sync_session();
        parse_json(&text, path)
    }

    /// Map the portal's responses onto the family exit codes.
    ///
    /// An expired ServiceStack session answers with a `302` to `/Home` (the
    /// login page) rather than a `401`, so landing there is the real "session
    /// expired" signal.
    fn handle_text(
        &self,
        resp: reqwest::blocking::Response,
        path: &str,
    ) -> Result<String, CliError> {
        let status = resp.status();
        let final_url = resp.url().to_string();
        let landed_on_login = final_url.ends_with("/Home") || final_url.contains("/Home?");
        // A request that *asked* for /Home hasn't been bounced anywhere.
        if landed_on_login && !path.starts_with("/Home") {
            return Err(CliError::Auth(
                "portal session expired — run `wabhoa auth login`".into(),
            ));
        }
        if matches!(status.as_u16(), 401 | 403) {
            return Err(CliError::Auth(format!(
                "portal returned {} for {path} — run `wabhoa auth login`",
                status.as_u16()
            )));
        }
        if status.as_u16() == 404 {
            return Err(CliError::NotFound(format!("{path} (HTTP 404)")));
        }
        let text = resp
            .text()
            .map_err(|e| CliError::Upstream(format!("reading response body: {e}")))?;
        if !status.is_success() {
            return Err(CliError::Upstream(format!(
                "portal HTTP {} for {path}{}",
                status.as_u16(),
                body_hint(&text)
            )));
        }
        // The login page served with a 200 also means the session is gone.
        if !path.starts_with("/Home") && text.contains("id=\"txtPassword\"") {
            return Err(CliError::Auth(
                "portal served the login page — session expired; run `wabhoa auth login`".into(),
            ));
        }
        Ok(text)
    }
}

/// Parse a response body as JSON, reporting an HTML body as session expiry.
fn parse_json(text: &str, path: &str) -> Result<Value, CliError> {
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(text).map_err(|_| {
        if text.trim_start().starts_with('<') {
            CliError::Auth(
                "portal returned HTML instead of JSON (session expired) — run `wabhoa auth login`"
                    .into(),
            )
        } else {
            CliError::Upstream(format!(
                "portal returned non-JSON for {path} (first bytes: {:?})",
                text.chars().take(60).collect::<String>()
            ))
        }
    })
}

/// Pull a short human hint out of an error body for error messages.
fn body_hint(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        for ptr in ["/ResponseStatus/Message", "/message", "/error"] {
            if let Some(m) = v.pointer(ptr).and_then(Value::as_str) {
                if !m.is_empty() {
                    return format!(" — {m}");
                }
            }
        }
    }
    format!(" — {}", trimmed.chars().take(120).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portal() -> Portal {
        Portal::new("https://pay.example.com").unwrap()
    }

    #[test]
    fn urls_join_base_and_path() {
        let p = portal();
        assert_eq!(p.url("/Dashboard"), "https://pay.example.com/Dashboard");
        assert_eq!(p.url("Dashboard"), "https://pay.example.com/Dashboard");
        assert_eq!(p.url("https://other/y"), "https://other/y");
    }

    #[test]
    fn invalid_base_url_is_a_usage_error() {
        assert!(Portal::new("not a url").is_err());
    }

    #[test]
    fn session_bundle_round_trips_through_the_jar() {
        let p = portal();
        p.seed_bundle("ss-id=abc; ss-pid=def; opayUC=ghi");
        let bundle = p.session_bundle().expect("bundle present");
        let raw = bundle.expose();
        assert!(raw.contains("ss-id=abc"), "got {raw}");
        assert!(raw.contains("ss-pid=def"), "got {raw}");
        assert!(raw.contains("opayUC=ghi"), "got {raw}");
    }

    #[test]
    fn session_bundle_is_absent_without_the_authenticating_cookie() {
        let p = portal();
        // Analytics cookies alone are not a session.
        p.seed_bundle("_ga=1; MgmtCoId=7001");
        assert!(p.session_bundle().is_none());
    }

    #[test]
    fn sync_is_a_noop_without_a_keychain_binding() {
        // `Portal::new` is the login-time client: no cached session to write
        // back to, so syncing must do nothing rather than panic.
        let p = portal();
        assert!(p.sync.is_none());
        p.seed_bundle("ss-id=fresh");
        p.sync_session();
        assert_eq!(p.cookie("ss-id").as_deref(), Some("fresh"));
    }

    #[test]
    fn html_where_json_was_expected_reads_as_expired() {
        let err = parse_json("<!doctype html><html>", "/x").unwrap_err();
        assert!(matches!(err, CliError::Auth(_)), "got {err:?}");
    }

    #[test]
    fn empty_body_parses_as_null() {
        assert_eq!(parse_json("   ", "/x").unwrap(), Value::Null);
    }

    #[test]
    fn non_json_non_html_is_an_upstream_error() {
        let err = parse_json("kaboom", "/x").unwrap_err();
        assert!(matches!(err, CliError::Upstream(_)), "got {err:?}");
    }

    #[test]
    fn body_hint_prefers_the_servicestack_message() {
        assert_eq!(body_hint(""), "");
        assert_eq!(
            body_hint(r#"{"ResponseStatus":{"Message":"nope"}}"#),
            " — nope"
        );
        assert_eq!(body_hint("plain text"), " — plain text");
        // " — " plus 120 truncated chars.
        assert_eq!(body_hint(&"x".repeat(200)).chars().count(), 123);
    }
}
