//! `wabhoa payments` — assessment payment history.

use clap::Subcommand;
use pk_cli_core::{output, CliError};
use serde_json::{json, Value};

use super::{emit, items, note_empty, table_view, Ctx, HISTORY_PAGE, HISTORY_SEARCH};
use crate::dates::RangeArgs;
use crate::parse;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List payments, newest first.
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Show one payment by its transaction number.
    Get {
        /// Transaction number from `payments list`.
        transaction_number: String,
    },
}

#[derive(clap::Args, Debug)]
pub struct ListArgs {
    #[command(flatten)]
    pub range: RangeArgs,

    /// Only payments with this status, e.g. `Processed` or `Pending`.
    #[arg(long)]
    pub status: Option<String>,

    /// Only payments against this property account number.
    #[arg(long, value_name = "ACCOUNT")]
    pub account: Option<String>,

    /// Maximum payments to return.
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
}

pub fn run(ctx: &Ctx, cmd: &Cmd) -> Result<(), CliError> {
    // Validate before opening a session, so bad args never hit the keychain.
    if let Cmd::List(args) = cmd {
        args.range.resolve()?;
    }

    match cmd {
        Cmd::List(args) => {
            let (start, end) = args.range.resolve()?;
            let status = args.status.as_deref();
            // Both reads sit inside one retry: the site-user ID comes off the
            // history page, so a session that lapses between the two would
            // otherwise fail the search with a stale ID in hand.
            let mut found = ctx.read(|c| search(c, start.clone(), end.clone(), status))?;

            // The portal ignores an account filter in the search body, so it
            // is applied here rather than pretended to be a server-side one.
            if let Some(account) = &args.account {
                found.retain(|p| {
                    p.get("account_number").and_then(Value::as_str) == Some(account.as_str())
                });
            }
            let total = found.len();
            found.truncate(args.limit as usize);
            if total > found.len() && !ctx.common.quiet {
                eprintln!(
                    "showing {} of {total} payments — raise --limit to see the rest",
                    found.len()
                );
            }
            if found.is_empty() {
                note_empty(ctx, "payments");
            }

            emit(ctx, "payment-list", json!({ "payments": found }), |v| {
                output::table(&table_view(
                    &items(v, "payments"),
                    &[
                        "payment_date",
                        "property",
                        "amount",
                        "status",
                        "type",
                        "transaction_number",
                    ],
                ));
            });
            Ok(())
        }
        Cmd::Get { transaction_number } => {
            let found = ctx.read(|c| search(c, None, None, None))?;
            let payment = found
                .iter()
                .find(|p| {
                    p.get("transaction_number").and_then(Value::as_str)
                        == Some(transaction_number.as_str())
                })
                .ok_or_else(|| {
                    CliError::NotFound(format!("no payment with transaction number {transaction_number} — run `wabhoa payments list`"))
                })?;
            emit(ctx, "payment", payment.clone(), output::render);
            Ok(())
        }
    }
}

/// Fetch payment history: read the site-user ID off the history page, then
/// post the search. Kept together so both share one session attempt.
fn search(
    client: &crate::client::Portal,
    start: Option<String>,
    end: Option<String>,
    status: Option<&str>,
) -> Result<Vec<Value>, CliError> {
    let site_user = parse::site_user_id(&client.get_text(HISTORY_PAGE)?).ok_or_else(|| {
        CliError::Upstream(
            "payment history page carried no site-user ID — portal markup changed?".into(),
        )
    })?;
    let body = search_body(&site_user, start, end, status);
    Ok(parse::payments(&client.post_json(HISTORY_SEARCH, &body)?))
}

/// Build the search body. Every field is required by the endpoint's model
/// binder — omitting one yields a validation error rather than a wildcard —
/// so unused filters are sent as empty strings, exactly as the portal does.
fn search_body(
    site_user: &str,
    start: Option<String>,
    end: Option<String>,
    status: Option<&str>,
) -> Value {
    json!({
        "PropertyId": "",
        "PaymentDate": Value::Null,
        "PaymentDateMin": start.map_or(Value::Null, Value::String),
        "PaymentDateMax": end.map_or(Value::Null, Value::String),
        "PaymentAmount": "",
        "PaymentAmountMin": "",
        "PaymentAmountMax": "",
        "PaymentStatus": status.unwrap_or(""),
        "TransactionNumber": "",
        "SiteUserLogin": site_user,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_body_sends_empty_strings_not_nulls_for_unused_filters() {
        let b = search_body("7654321", None, None, None);
        assert_eq!(b["SiteUserLogin"], "7654321");
        assert_eq!(b["PaymentStatus"], "");
        assert_eq!(b["PropertyId"], "");
        // Date bounds are the exception: the binder wants null, not "".
        assert_eq!(b["PaymentDateMin"], Value::Null);
    }

    #[test]
    fn search_body_carries_the_portal_date_format() {
        let b = search_body(
            "1",
            Some("01/01/2026".into()),
            Some("12/31/2026".into()),
            Some("Processed"),
        );
        assert_eq!(b["PaymentDateMin"], "01/01/2026");
        assert_eq!(b["PaymentDateMax"], "12/31/2026");
        assert_eq!(b["PaymentStatus"], "Processed");
    }
}
