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

    /// Fetch a published statement packet's PDF bytes.
    ///
    /// The portal's own `DownloadStatement(fileName, fileAlias)` handler POSTs
    /// `{"FileName": <fileName>}` to `/Statements/GetStatementByteArray` and
    /// reads back `{"IsSuccessful": true, "File": "<base64>"}` — the response
    /// is a JSON envelope carrying the PDF as base64 text, not a raw binary
    /// stream. A `IsSuccessful: false` surfaces the portal's own
    /// `StatusMessage` rather than a bare "download failed".
    ///
    /// Nothing about the HTTP status is trusted. The response is accepted only
    /// after two positive checks: the envelope says `IsSuccessful: true` with a
    /// `File` string ([`statement_error`]), and the decoded bytes carry a
    /// `%PDF-` header ([`pdf_error`]). See those for why each is load-bearing.
    ///
    /// A read: it fetches an already-published file and is absent from the
    /// [`crate::writes`] catalog.
    pub fn download_statement(&self, file_name: &str) -> Result<Vec<u8>, CliError> {
        let path = crate::commands::STATEMENT_BYTES;
        let body = json!({ "FileName": file_name });
        let payload = self.post_json(path, &body)?;
        if let Some(err) = statement_error(&payload, file_name) {
            return Err(CliError::Upstream(err));
        }
        let encoded = payload
            .get("File")
            .and_then(Value::as_str)
            .expect("statement_error rejected a missing File");
        let bytes = base64_decode(encoded.trim()).ok_or_else(|| {
            CliError::Upstream(format!(
                "statement download returned an undecodable body for {file_name:?}"
            ))
        })?;
        // The success signal. Everything above this line can be true of a
        // response that is not a statement.
        if let Some(err) = pdf_error(&bytes, file_name) {
            return Err(CliError::Upstream(err));
        }
        Ok(bytes)
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
    /// Session expiry is the subtle part: the portal never answers `401` for
    /// it. See [`expired_session`] for the tells.
    fn handle_text(
        &self,
        resp: reqwest::blocking::Response,
        path: &str,
    ) -> Result<String, CliError> {
        let status = resp.status();
        let final_path = resp.url().path().to_string();
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
        if let Some(tell) = expired_session(path, &final_path, &text) {
            return Err(CliError::Auth(format!("{tell} — run `wabhoa auth login`")));
        }
        Ok(text)
    }
}

/// Whether a response is the portal's way of saying "your session is gone",
/// and which tell gave it away.
///
/// The portal never answers `401` for an expired session. It has three
/// distinct ways of refusing instead, all observed on a genuinely expired
/// session — and a client that checks only one of them reports "you have no
/// notifications" to a logged-out user, which is worse than an error:
///
/// 1. **Bounced to the login page.** The redirect *prefixes* the requested
///    path with `/Home`, so `/Properties/StatementHistory` comes back as
///    `/Home/Properties/StatementHistory` — matching on the URL merely
///    *ending* in `/Home` misses every one of these but the bare case.
/// 2. **The login form served in place of the page**, with a `200`.
/// 3. **An empty `200` with no redirect at all** — how `/Account/Profile` and
///    `/Notifications/List` refuse. Nothing this CLI requests legitimately
///    answers with an empty body, so a blank page is always this.
///
/// A request that *asked* for `/Home` hasn't been bounced anywhere, so the
/// first two tells exempt it.
fn expired_session(requested: &str, final_path: &str, body: &str) -> Option<&'static str> {
    let asked_for_home = requested.starts_with("/Home");
    if !asked_for_home && (final_path == "/Home" || final_path.starts_with("/Home/")) {
        return Some("portal session expired");
    }
    if !asked_for_home && body.contains("id=\"txtPassword\"") {
        return Some("portal served the login page (session expired)");
    }
    if body.trim().is_empty() {
        return Some("portal returned an empty page (session expired)");
    }
    None
}

/// Log in and cache the resulting session in the keychain.
///
/// Shared by the explicit `auth login` and the automatic recovery in
/// [`crate::commands::Ctx::read`], so both prove the session works before
/// storing it: a login that can't actually read leaves no broken session
/// behind, and the cookies stored are the ones that survived that read, since
/// the portal may rotate them mid-flight.
pub fn establish_session(
    cfg: &Config,
    creds: &CredentialStore,
    username: &str,
    password: &Secret,
) -> Result<(), CliError> {
    let portal = Portal::new(cfg.base_url())?;
    portal.login(username, password)?;
    portal.get_text(crate::commands::HISTORY_PAGE)?;
    let session = portal.session_bundle().ok_or_else(|| {
        CliError::Upstream("login succeeded but the portal issued no session cookie".into())
    })?;
    creds.set(SESSION_ACCOUNT, &session)
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

/// Interpret a `/Statements/GetStatementByteArray` response envelope, returning
/// the human message when the payload is *not* a usable statement.
///
/// Pure so every response shape observed — the portal's `IsSuccessful: false`
/// with and without a `StatusMessage`, a success envelope missing the `File`
/// field entirely, and the ordinary success case — is covered by unit cases
/// rather than a live HTTP call. Mirrors [`expired_session`] and [`body_hint`].
///
/// A `None` return means the envelope is a real success: `IsSuccessful: true`
/// **and** `File` is a string. Callers may then decode `File` without a
/// second existence check.
fn statement_error(payload: &Value, file_name: &str) -> Option<String> {
    let ok = payload.get("IsSuccessful").and_then(Value::as_bool) == Some(true);
    if !ok {
        // The portal's own `StatusMessage` is more useful than a bare
        // "download failed" — surface it when it's there.
        let msg = payload
            .get("StatusMessage")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("no statement returned for {file_name:?}"));
        return Some(format!("statement download refused: {msg}"));
    }
    // `IsSuccessful: true` without a `File` field is the portal drifting.
    // Treat it as an upstream error rather than an empty PDF.
    if payload.get("File").and_then(Value::as_str).is_none() {
        return Some(format!(
            "statement download succeeded but carried no `File` field for {file_name:?}"
        ));
    }
    None
}

/// Confirm that decoded statement bytes really are a PDF, returning the human
/// message when they are not.
///
/// **This is the download path's only positive success signal**, which is why
/// it exists at all. Every other tell this portal offers can be true of a
/// response that is not a statement: it answers `200` for refusals, reports
/// failure inside the envelope rather than on the status line, and — the case
/// this guards — can hand back an HTML error or login page wrapped in the very
/// same base64 `File` field a real statement arrives in. Without this check the
/// CLI would decode that page, write it to disk under a `.pdf` name, and print
/// "Saved …" for a file no PDF reader can open.
///
/// A PDF is identified by the `%PDF-` header from its spec. The scan tolerates
/// a short preamble, because readers do and some generators emit one, but stays
/// inside the first kilobyte so an HTML page that merely *mentions* the string
/// further down cannot pass.
///
/// Returning `Upstream` (exit 5) rather than `Auth` (exit 3) even for the HTML
/// case is deliberate: genuine session expiry is caught upstream of here by
/// [`expired_session`] and [`parse_json`], which re-auth and retry. Reaching
/// this function means the portal claimed success and then sent a non-PDF —
/// that is the portal misbehaving, and retrying the login would not fix it.
fn pdf_error(bytes: &[u8], file_name: &str) -> Option<String> {
    const MAGIC: &[u8] = b"%PDF-";
    let window = &bytes[..bytes.len().min(1024)];
    if window.windows(MAGIC.len()).any(|w| w == MAGIC) {
        return None;
    }
    if bytes.is_empty() {
        return Some(format!(
            "portal returned an empty body for statement {file_name:?} instead of a PDF"
        ));
    }
    let first_visible = window
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| window[i]);
    if first_visible == Some(b'<') {
        return Some(format!(
            "portal returned an HTML page instead of a PDF for statement {file_name:?} \
             — the session most likely lapsed mid-download; run `wabhoa auth login` and retry"
        ));
    }
    Some(format!(
        "portal returned {} bytes for statement {file_name:?} that are not a PDF \
         (no %PDF- header) — nothing was written",
        bytes.len()
    ))
}

/// Base64 decoder covering both alphabets, padding optional. Deliberately
/// local: the only base64 this crate touches is the `File` field on a
/// statement-download response, and pulling in a dependency for that would be
/// out of scale with the payload.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const fn value(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'-' | b'+' => Some(62),
            b'_' | b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for b in input.bytes() {
        // Whitespace inside base64 is common when a server pretty-prints; skip
        // it rather than fail. `=` ends the useful payload.
        if b == b'=' {
            break;
        }
        if b.is_ascii_whitespace() {
            continue;
        }
        acc = (acc << 6) | value(b)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
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

    /// Every one of these is a response shape observed from the live portal on
    /// a genuinely expired session (2026-08-07). Before this test, only the
    /// bare `/Home` redirect and the login-form body were caught: `wabhoa
    /// notifications list` answered "no notifications found" with exit 0 to a
    /// logged-out user, and `wabhoa profile` printed nothing and exited 0.
    #[test]
    fn every_observed_expiry_tell_is_detected() {
        // 1. Bounced to the bare login page (/Payment/MakePayment).
        assert!(expired_session("/Payment/MakePayment", "/Home", "<html>…</html>").is_some());
        // 1b. Bounced with the requested path *prefixed* onto /Home — the
        //     shape that `ends_with("/Home")` missed entirely.
        assert!(expired_session(
            "/Properties/StatementHistory",
            "/Home/Properties/StatementHistory",
            "<html>…</html>"
        )
        .is_some());
        // 2. The login form served in place of the page, HTTP 200.
        assert!(expired_session(
            "/Properties/StatementHistory",
            "/Properties/StatementHistory",
            r#"<input id="txtPassword" type="password">"#
        )
        .is_some());
        // 3. An empty 200 with no redirect (/Account/Profile,
        //    /Notifications/List). This is the one that silently returned
        //    "no data" instead of "logged out".
        assert!(expired_session("/Account/Profile", "/Account/Profile", "").is_some());
        assert!(expired_session("/Notifications/List", "/Notifications/List", "   \n").is_some());
    }

    #[test]
    fn a_live_response_is_not_mistaken_for_expiry() {
        assert!(expired_session(
            "/Payment/MakePayment",
            "/Payment/MakePayment",
            r#"<select id="idPropertyMember"><option value="1">x</option></select>"#
        )
        .is_none());
        // JSON from a search endpoint.
        assert!(expired_session(
            "/Payment/PaymentHistorySearch",
            "/Payment/PaymentHistorySearch",
            "{}"
        )
        .is_none());
        // Asking for the login page itself is not "being bounced" to it, and
        // its body legitimately carries the password field.
        assert!(expired_session("/Home", "/Home", r#"<input id="txtPassword">"#).is_none());
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
    fn statement_error_surfaces_the_portals_own_message_when_present() {
        // The observed shape on the live portal — the endpoint reports the
        // failure through the envelope, not an HTTP error, so the message
        // must reach the user rather than be replaced with a generic string.
        let payload = json!({
            "IsSuccessful": false,
            "StatusCode": -1100,
            "StatusMessage": "There was an error trying to perform the requested action.",
        });
        let err = statement_error(&payload, "9001.pdf").expect("error expected");
        assert!(err.starts_with("statement download refused:"), "{err}");
        assert!(err.contains("There was an error"), "{err}");
    }

    #[test]
    fn statement_error_falls_back_when_the_portal_omits_a_status_message() {
        // A refusal with no message (whitespace-only is treated the same) must
        // still name the file being asked for, so the diagnostic is actionable.
        for payload in [
            json!({ "IsSuccessful": false }),
            json!({ "IsSuccessful": false, "StatusMessage": "" }),
            json!({ "IsSuccessful": false, "StatusMessage": "   " }),
        ] {
            let err = statement_error(&payload, "9001.pdf").expect("error expected");
            assert!(err.contains("no statement returned for"), "{err}");
            assert!(err.contains("9001.pdf"), "{err}");
        }
    }

    #[test]
    fn statement_error_rejects_a_success_envelope_missing_the_file_field() {
        // A `true`-but-no-`File` envelope would previously slip through and
        // then blow up in the decoder — treat it as an upstream drift here,
        // with a message that points at the missing field.
        let payload = json!({ "IsSuccessful": true });
        let err = statement_error(&payload, "9001.pdf").expect("error expected");
        assert!(err.contains("no `File` field"), "{err}");
        assert!(err.contains("9001.pdf"), "{err}");

        // A non-string `File` (e.g. `null`) is treated the same — the decoder
        // needs a string in hand, not any JSON value.
        let payload = json!({ "IsSuccessful": true, "File": null });
        assert!(statement_error(&payload, "9001.pdf").is_some());
    }

    #[test]
    fn statement_error_passes_a_real_success_envelope_through() {
        // The only shape a caller may safely decode from.
        let payload = json!({ "IsSuccessful": true, "File": "SGVsbG8=" });
        assert!(statement_error(&payload, "9001.pdf").is_none());
    }

    #[test]
    fn pdf_error_accepts_a_real_pdf_header() {
        // The whole point of the check: a genuine statement passes.
        assert!(pdf_error(b"%PDF-1.7\n1 0 obj\n", "9001.pdf").is_none());
        // A short preamble before the header is tolerated, as readers do.
        let mut padded = b"\n\n".to_vec();
        padded.extend_from_slice(b"%PDF-1.4 rest of file");
        assert!(pdf_error(&padded, "9001.pdf").is_none());
    }

    #[test]
    fn pdf_error_rejects_an_html_page_wrapped_in_a_success_envelope() {
        // The auth-expiry trap: the portal claims success and base64-wraps a
        // login page. Writing this to disk as a `.pdf` and reporting "Saved"
        // is the exact lie this check exists to prevent.
        let login_page = br#"<!DOCTYPE html><html><body><input id="txtPassword"></body></html>"#;
        let err = pdf_error(login_page, "9001.pdf").expect("HTML must be rejected");
        assert!(err.contains("HTML page instead of a PDF"), "{err}");
        assert!(err.contains("auth login"), "{err}");
        // Leading whitespace before the markup must not smuggle it past.
        let err = pdf_error(b"  \n <html>nope</html>", "9001.pdf").expect("HTML must be rejected");
        assert!(err.contains("HTML page instead of a PDF"), "{err}");
    }

    #[test]
    fn pdf_error_rejects_an_empty_or_arbitrary_body() {
        let err = pdf_error(b"", "9001.pdf").expect("empty must be rejected");
        assert!(err.contains("empty body"), "{err}");

        let err = pdf_error(b"not a pdf at all", "9001.pdf").expect("garbage must be rejected");
        assert!(err.contains("not a PDF"), "{err}");
        // The message says nothing was saved, because nothing was.
        assert!(err.contains("nothing was written"), "{err}");
    }

    #[test]
    fn pdf_error_ignores_a_late_magic_string() {
        // An HTML page that happens to contain the literal `%PDF-` well past
        // the header window must not be accepted as a PDF.
        let mut sneaky = b"<html><body>".to_vec();
        sneaky.extend(std::iter::repeat_n(b' ', 2000));
        sneaky.extend_from_slice(b"%PDF-1.7");
        let err = pdf_error(&sneaky, "9001.pdf").expect("late magic must not count");
        assert!(err.contains("HTML page instead of a PDF"), "{err}");
    }

    #[test]
    fn base64_decodes_padded_and_unpadded_input() {
        // Standard alphabet, both with and without padding, and with the
        // whitespace a pretty-printer might insert.
        assert_eq!(base64_decode("SGVsbG8=").unwrap(), b"Hello");
        assert_eq!(base64_decode("SGVsbG8").unwrap(), b"Hello");
        assert_eq!(base64_decode("SGV s bG8=").unwrap(), b"Hello");
        // URL-safe alphabet.
        assert_eq!(
            base64_decode("-_+/").unwrap(),
            base64_decode("-_-_").unwrap()
        );
        // A stray non-alphabet byte fails, so a garbled body doesn't silently
        // become empty output.
        assert!(base64_decode("****").is_none());
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
