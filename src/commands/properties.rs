//! `wabhoa properties` — the units this login pays assessments for.

use clap::Subcommand;
use pk_cli_core::{output, CliError};
use serde_json::{json, Value};

use super::{emit, items, note_empty, table_view, Ctx, PAYMENT_OPTIONS, PAYMENT_PAGE};
use crate::parse;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List properties with their association and account identity.
    #[command(visible_alias = "ls")]
    List,
    /// Show one property, including the association's published balance.
    Get {
        /// Property ID from `properties list`.
        property_id: String,
    },
}

pub fn run(ctx: &Ctx, cmd: &Cmd) -> Result<(), CliError> {
    let properties = ctx.read(|c| Ok(parse::properties(&c.get_text(PAYMENT_PAGE)?)))?;

    match cmd {
        Cmd::List => {
            if properties.is_empty() {
                note_empty(ctx, "properties");
            }
            emit(
                ctx,
                "property-list",
                json!({ "properties": properties }),
                |v| {
                    output::table(&table_view(
                        &items(v, "properties"),
                        &[
                            "id",
                            "address",
                            "association_id",
                            "account_number",
                            "echeck_fee",
                            "debit_fee",
                        ],
                    ));
                },
            );
            Ok(())
        }
        Cmd::Get { property_id } => {
            let found = properties
                .iter()
                .find(|p| p.get("id").and_then(Value::as_str) == Some(property_id.as_str()))
                .ok_or_else(|| {
                    CliError::NotFound(format!(
                        "no property with id {property_id} — run `wabhoa properties list`"
                    ))
                })?;

            // Merge in the association's payment options, which is where a
            // published balance and the next assessment date live.
            let mut detail = found.clone();
            if let Some(options) = ctx.read(|c| fetch_options(c, found))? {
                if let (Value::Object(d), Value::Object(b)) =
                    (&mut detail, parse::balance(&options))
                {
                    d.extend(b);
                }
            }
            emit(ctx, "property", detail, output::render);
            Ok(())
        }
    }
}

/// Ask the portal for one property's payment options. Returns `None` when the
/// property row lacks the association identity the endpoint keys on.
fn fetch_options(
    client: &crate::client::Portal,
    property: &Value,
) -> Result<Option<Value>, CliError> {
    let field = |k: &str| property.get(k).and_then(Value::as_str).map(str::to_string);
    let (Some(cmc), Some(assoc), Some(account)) = (
        field("management_company_id"),
        field("association_id"),
        field("account_number"),
    ) else {
        return Ok(None);
    };
    let body = json!({
        "ManagementCompanyId": cmc,
        "AssociationId": assoc,
        "PropertyAccountNumber": account,
    });
    client.post_json(PAYMENT_OPTIONS, &body).map(Some)
}
