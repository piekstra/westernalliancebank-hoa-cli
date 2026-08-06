//! `wabhoa methods` — saved bank accounts and cards.
//!
//! Only the portal's own masked view is reported: a name, a last-four, and the
//! method's ID. Full account numbers are never exposed by the portal, and this
//! CLI never asks for them.

use clap::Subcommand;
use pk_cli_core::{output, CliError};
use serde_json::json;

use super::{emit, items, note_empty, table_view, Ctx, PAYMENT_PAGE};
use crate::parse;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List saved payment methods.
    #[command(visible_alias = "ls")]
    List,
}

pub fn run(ctx: &Ctx, cmd: &Cmd) -> Result<(), CliError> {
    let Cmd::List = cmd;
    let methods = parse::payment_methods(&ctx.client()?.get_text(PAYMENT_PAGE)?);
    if methods.is_empty() {
        note_empty(ctx, "payment methods");
    }
    emit(
        ctx,
        "payment-method-list",
        json!({ "payment_methods": methods }),
        |v| {
            output::table(&table_view(
                &items(v, "payment_methods"),
                &["id", "name", "mask", "type", "account_type"],
            ));
        },
    );
    Ok(())
}
