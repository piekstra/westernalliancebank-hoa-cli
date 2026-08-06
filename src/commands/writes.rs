//! `wabhoa writes` — the portal's mutating endpoints, none of which this CLI
//! implements.
//!
//! Named `writes` rather than `capabilities` because `wabhoa info` already uses
//! "capabilities" for the list of commands this CLI *does* offer; conflating
//! the two would mislead exactly the automated readers `info` exists for.
//!
//! Needs no session: it reports this CLI's own scope, not account state.

use pk_cli_core::{output, CliError};
use serde_json::{json, Value};

use super::{emit, items, table_view, Ctx};
use crate::writes::CAPABILITIES;

pub fn run(ctx: &Ctx) -> Result<(), CliError> {
    let rows: Vec<Value> = CAPABILITIES
        .iter()
        .map(|c| {
            json!({
                "method": c.method,
                "path": c.path,
                "category": c.category.as_str(),
                "description": c.description,
                "implemented": false,
            })
        })
        .collect();

    emit(
        ctx,
        "write-capability-list",
        json!({ "writes": rows }),
        |v| {
            println!(
                "Portal write endpoints this CLI does NOT implement ({} total).",
                CAPABILITIES.len()
            );
            println!("Every command in this CLI is a read; nothing below is wired up.\n");
            output::table(&table_view(
                &items(v, "writes"),
                &["category", "method", "path", "description"],
            ));
        },
    );
    Ok(())
}
