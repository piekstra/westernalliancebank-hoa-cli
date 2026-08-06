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

use pk_cli_core::{output, CliError, CommonArgs};
use pk_cli_secrets::CredentialStore;
use serde_json::Value;

use crate::client::Portal;
use crate::config::Config;

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
}

/// Emit a DTO: tagged payload in JSON mode, rendered view in text mode.
pub fn emit(ctx: &Ctx, schema: &str, payload: Value, text: impl FnOnce(&Value)) {
    if ctx.common.json {
        let mut tagged = serde_json::Map::new();
        tagged.insert("schema".into(), Value::String(format!("{schema}/v1")));
        match payload {
            Value::Object(m) => tagged.extend(m),
            other => {
                tagged.insert("data".into(), other);
            }
        }
        output::json(&Value::Object(tagged));
    } else {
        text(&payload);
    }
}

/// Pull selected columns out of an array of objects for table rendering.
pub fn table_view(items: &[Value], columns: &[&str]) -> Vec<Value> {
    items
        .iter()
        .map(|item| {
            let mut row = serde_json::Map::new();
            for col in columns {
                if let Some(v) = item.get(*col) {
                    row.insert((*col).to_string(), v.clone());
                }
            }
            Value::Object(row)
        })
        .collect()
}

/// Read an array field out of an emitted payload for the text renderer.
pub fn items(v: &Value, key: &str) -> Vec<Value> {
    v.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
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
