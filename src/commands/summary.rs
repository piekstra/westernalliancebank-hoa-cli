//! `wabhoa summary` — the dashboard in one call: what's owed, what's coming,
//! and what was paid recently.

use pk_cli_core::{output, CliError};
use serde_json::{json, Value};

use super::{
    emit, table_view, Ctx, DASHBOARD_CONTENT, HISTORY_PAGE, HISTORY_SEARCH, PAYMENT_OPTIONS,
    PAYMENT_PAGE,
};
use crate::parse;

/// How many recent payments the overview shows.
const RECENT: usize = 5;

pub fn run(ctx: &Ctx) -> Result<(), CliError> {
    let client = ctx.client()?;

    let mut properties = parse::properties(&client.get_text(PAYMENT_PAGE)?);
    // Fold each association's published balance into its property row.
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

    let history_page = client.get_text(HISTORY_PAGE)?;
    let mut recent = match parse::site_user_id(&history_page) {
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
