//! `wabhoa statements` — association statement packets, when published.
//!
//! Many associations publish none, in which case the portal renders "NO
//! STATEMENT DATA AVAILABLE" and this command returns an empty list.

use clap::Subcommand;
use pk_cli_core::{output, CliError};
use serde_json::json;

use super::{emit, items, note_empty, table_view, Ctx, STATEMENTS_PAGE};
use crate::parse;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List published statements.
    #[command(visible_alias = "ls")]
    List,
}

pub fn run(ctx: &Ctx, cmd: &Cmd) -> Result<(), CliError> {
    let Cmd::List = cmd;
    let statements = ctx.read(|c| Ok(parse::statements(&c.get_text(STATEMENTS_PAGE)?)))?;
    if statements.is_empty() {
        note_empty(ctx, "statements");
    }
    emit(
        ctx,
        "statement-list",
        json!({ "statements": statements }),
        |v| {
            output::table(&table_view(
                &items(v, "statements"),
                &["date", "description", "amount"],
            ));
        },
    );
    Ok(())
}
