//! `wabhoa summary` — the dashboard in one call: what's owed, what's coming,
//! and what was paid recently.

use pk_cli_core::{output, CliError};
use serde_json::{json, Value};

use super::{
    emit, table_view, Ctx, DASHBOARD_CONTENT, HISTORY_PAGE, HISTORY_SEARCH, PAYMENT_OPTIONS,
    PAYMENT_PAGE,
};
use crate::client::Portal;
use crate::parse;

/// How many recent payments the overview shows.
const RECENT: usize = 5;

pub fn run(ctx: &Ctx) -> Result<(), CliError> {
    // The whole overview is one retry unit. Splitting it would let a session
    // that lapses partway through render a half-empty dashboard — properties
    // present, payments silently missing — which reads as "no payments".
    let (properties, scheduled, recent) = ctx.read(collect)?;

    let payload = json!({
        "properties": properties,
        "scheduled_payments": scheduled,
        "recent_payments": recent,
    });

    emit(ctx, "summary", payload, |v| {
        section(
            "Properties",
            v,
            "properties",
            &[
                "address",
                "association_id",
                "account_number",
                "balance",
                "balance_published",
            ],
        );
        section(
            "Scheduled payments",
            v,
            "scheduled_payments",
            &["property", "next_payment_date", "frequency", "amount"],
        );
        section(
            "Recent payments",
            v,
            "recent_payments",
            &["payment_date", "property", "amount", "status"],
        );
    });
    Ok(())
}

/// Gather every part of the overview: properties with balances folded in,
/// scheduled payments, and the most recent payments.
#[allow(clippy::type_complexity)]
fn collect(client: &Portal) -> Result<(Vec<Value>, Vec<Value>, Vec<Value>), CliError> {
    let mut properties = parse::properties(&client.get_text(PAYMENT_PAGE)?);
    for property in &mut properties {
        let field = |k: &str| property.get(k).and_then(Value::as_str).map(str::to_string);
        let (Some(cmc), Some(assoc), Some(account)) = (
            field("management_company_id"),
            field("association_id"),
            field("account_number"),
        ) else {
            continue;
        };
        let body = json!({
            "ManagementCompanyId": cmc,
            "AssociationId": assoc,
            "PropertyAccountNumber": account,
        });
        let options = client.post_json(PAYMENT_OPTIONS, &body)?;
        if let (Value::Object(p), Value::Object(b)) = (property, parse::balance(&options)) {
            p.extend(b);
        }
    }

    let scheduled = parse::scheduled_payments(&client.get_text(DASHBOARD_CONTENT)?);

    let mut recent = match parse::site_user_id(&client.get_text(HISTORY_PAGE)?) {
        Some(site_user) => {
            let body = json!({
                "PropertyId": "", "PaymentDate": Value::Null,
                "PaymentDateMin": Value::Null, "PaymentDateMax": Value::Null,
                "PaymentAmount": "", "PaymentAmountMin": "", "PaymentAmountMax": "",
                "PaymentStatus": "", "TransactionNumber": "", "SiteUserLogin": site_user,
            });
            parse::payments(&client.post_json(HISTORY_SEARCH, &body)?)
        }
        None => Vec::new(),
    };
    recent.truncate(RECENT);

    Ok((properties, scheduled, recent))
}

/// Render one titled block of the overview, or say it's empty.
fn section(title: &str, payload: &Value, key: &str, columns: &[&str]) {
    println!("{title}:");
    let rows = super::items(payload, key);
    if rows.is_empty() {
        println!("  (none)");
    } else {
        output::table(&table_view(&rows, columns));
    }
    println!();
}
