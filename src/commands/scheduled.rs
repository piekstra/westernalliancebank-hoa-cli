//! `wabhoa scheduled` — recurring payments the portal will make automatically.

use clap::Subcommand;
use pk_cli_core::{output, CliError};
use serde_json::json;

use super::{emit, items, note_empty, table_view, Ctx, DASHBOARD_CONTENT};
use crate::parse;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List scheduled payments.
    #[command(visible_alias = "ls")]
    List,
}

pub fn run(ctx: &Ctx, cmd: &Cmd) -> Result<(), CliError> {
    let Cmd::List = cmd;
    let dashboard = ctx.read(|c| c.get_text(DASHBOARD_CONTENT))?;
    let scheduled = parse::scheduled_payments(&dashboard);
    if scheduled.is_empty() {
        note_empty(ctx, "scheduled payments");
    }
    emit(
        ctx,
        "scheduled-payment-list",
        json!({ "scheduled_payments": scheduled }),
        |v| {
            output::table(&table_view(
                &items(v, "scheduled_payments"),
                &[
                    "id",
                    "property",
                    "next_payment_date",
                    "frequency",
                    "amount",
                    "type",
                ],
            ));
        },
    );
    Ok(())
}
