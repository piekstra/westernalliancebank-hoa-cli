//! Domain command modules. Each read emits a `schema`-tagged DTO in `--json`
//! mode and a shaped table/kv view in text mode.

pub mod api;
pub mod methods;
pub mod notifications;
pub mod payments;
pub mod profile;
pub mod properties;
pub mod scheduled;
pub mod statements;
pub mod summary;
pub mod writes;

use pk_cli_auth::reauth::with_reauth;
use pk_cli_core::{CliError, CommonArgs};
use pk_cli_secrets::CredentialStore;
use serde_json::Value;

use crate::client::{establish_session, Portal};
use crate::config::{Config, KEYCHAIN_ACCOUNT};

// §1.4's output contract now lives in the shared crate; re-exported so the
// command modules keep their short `use super::{emit, table_view}`.
pub use pk_cli_core::output::{rows_of as items, table_view};

/// Portal paths the reads share.
pub const PAYMENT_PAGE: &str = "/Payment/MakePayment";
pub const HISTORY_PAGE: &str = "/Payment/PaymentHistory";
pub const HISTORY_SEARCH: &str = "/Payment/PaymentHistorySearch";
pub const DASHBOARD_CONTENT: &str = "/DashboardContent";
pub const NOTIFICATIONS_PAGE: &str = "/Notifications/List";
pub const STATEMENTS_PAGE: &str = "/Properties/StatementHistory";
pub const PROFILE_PAGE: &str = "/Account/Profile";
pub const PAYMENT_OPTIONS: &str = "/Homeowner/PreSelectedPaymentOptions";

pub struct Ctx<'a> {
    pub common: &'a CommonArgs,
    pub cfg: &'a Config,
    pub creds: &'a CredentialStore,
}

impl Ctx<'_> {
    /// A portal session replayed from the keychain. Expiry surfaces as a
    /// `CliError::Auth` (exit 3) on the first read, pointing at `auth login`.
    pub fn client(&self) -> Result<Portal, CliError> {
        Portal::from_cached_session(self.cfg, self.creds)
    }

    /// Run a read against the portal, re-authenticating once if the session
    /// has lapsed.
    ///
    /// This portal expires sessions within a day, so without this nearly every
    /// first command of the day would fail with exit 3 for a session the CLI
    /// can mint again unattended — the password is already in the keychain and
    /// there is no second factor.
    ///
    /// **Reads only.** `op` runs twice on the recovery path; the retry rails
    /// live in `pk_cli_auth::reauth`.
    pub fn read<T>(&self, op: impl Fn(&Portal) -> Result<T, CliError>) -> Result<T, CliError> {
        with_reauth(
            // Rebuilt per attempt so the retry picks up the session that
            // `relogin` just wrote to the keychain.
            || op(&self.client()?),
            || self.relogin(),
        )
    }

    /// Mint a fresh session from the stored password.
    ///
    /// Declining is a normal outcome, not a failure to paper over: when
    /// `auto_login` is off or nothing is stored to log in with, this returns
    /// the same exit-3 guidance the user would have seen anyway.
    fn relogin(&self) -> Result<(), CliError> {
        let username = self.cfg.username();
        let password = self.creds.get(KEYCHAIN_ACCOUNT)?;
        if let Some(reason) = relogin_blocked(
            self.cfg.auto_login(),
            username.as_deref(),
            password.is_some(),
        ) {
            return Err(CliError::Auth(reason));
        }
        let (username, password) = (username.expect("checked"), password.expect("checked"));
        // Announced rather than silent: the password leaving the keychain is
        // worth one line of stderr, and it explains the extra round trip.
        if !self.common.quiet {
            eprintln!("session expired — re-authenticating as {username}");
        }
        establish_session(self.cfg, self.creds, &username, &password)
    }
}

/// Why an automatic re-login can't proceed, if it can't.
///
/// Split out from the action so the policy is testable without a keychain, and
/// so every refusal still tells the user the one command that fixes it.
fn relogin_blocked(auto_login: bool, username: Option<&str>, has_password: bool) -> Option<String> {
    if !auto_login {
        return Some("portal session expired — run `wabhoa auth login` (auto_login is off)".into());
    }
    if username.is_none() {
        return Some(
            "portal session expired and no username is configured — run \
             `wabhoa config set username <you@example.com>`, then `wabhoa auth login`"
                .into(),
        );
    }
    if !has_password {
        return Some(
            "portal session expired and no password is stored — run `wabhoa auth login`".into(),
        );
    }
    None
}

/// Emit a DTO, taking the `--json` flag off the context.
///
/// A thin adapter over `pk_cli_core::output::emit`, which owns the contract.
pub fn emit(ctx: &Ctx, schema: &str, payload: Value, text: impl FnOnce(&Value)) {
    pk_cli_core::output::emit(ctx.common.json, schema, payload, text)
}

/// Print a "nothing here" line to stderr, so stdout stays a clean (empty)
/// data stream for pipelines.
pub fn note_empty(ctx: &Ctx, what: &str) {
    if !ctx.common.quiet {
        eprintln!("no {what} found");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relogin_proceeds_when_everything_is_in_place() {
        assert_eq!(relogin_blocked(true, Some("you@example.com"), true), None);
    }

    #[test]
    fn relogin_declines_when_auto_login_is_off() {
        let why = relogin_blocked(false, Some("you@example.com"), true).expect("blocked");
        assert!(why.contains("auto_login is off"), "{why}");
        // Even when declining, the user is told what to run.
        assert!(why.contains("wabhoa auth login"), "{why}");
    }

    /// auto_login off wins over anything else, so turning it off is a real
    /// switch and not merely a preference the other branches can override.
    #[test]
    fn auto_login_off_takes_precedence_over_missing_credentials() {
        let why = relogin_blocked(false, None, false).expect("blocked");
        assert!(why.contains("auto_login is off"), "{why}");
    }

    #[test]
    fn relogin_declines_without_a_username_or_password() {
        let no_user = relogin_blocked(true, None, true).expect("blocked");
        assert!(no_user.contains("config set username"), "{no_user}");

        let no_pass = relogin_blocked(true, Some("you@example.com"), false).expect("blocked");
        assert!(no_pass.contains("no password is stored"), "{no_pass}");
        assert!(no_pass.contains("wabhoa auth login"), "{no_pass}");
    }

    use serde_json::json;

    #[test]
    fn table_view_selects_and_skips_missing() {
        let rows = table_view(
            &[json!({"a": 1, "b": 2, "c": 3}), json!({"a": 4})],
            &["a", "c"],
        );
        assert_eq!(rows[0], json!({"a": 1, "c": 3}));
        // Absent columns are omitted rather than nulled (SPEC: omit-don't-null).
        assert_eq!(rows[1], json!({"a": 4}));
    }

    #[test]
    fn items_tolerates_a_missing_or_wrong_typed_field() {
        assert_eq!(
            items(&json!({"xs": [1, 2]}), "xs"),
            vec![json!(1), json!(2)]
        );
        assert!(items(&json!({}), "xs").is_empty());
        assert!(items(&json!({"xs": "not an array"}), "xs").is_empty());
    }
}
