//! `wabhoa notifications` — payment notices the portal emailed out.

use clap::Subcommand;
use pk_cli_core::{output, CliError};
use serde_json::{json, Value};

use super::{emit, items, note_empty, table_view, Ctx, NOTIFICATIONS_PAGE};
use crate::parse;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List notifications, newest first.
    #[command(visible_alias = "ls")]
    List {
        /// Maximum notifications to return.
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Show one notification, including its message body.
    Get {
        /// Notification ID from `notifications list`.
        notification_id: String,
    },
}

pub fn run(ctx: &Ctx, cmd: &Cmd) -> Result<(), CliError> {
    let page = ctx.client()?.get_text(NOTIFICATIONS_PAGE)?;
    let all = parse::notifications(&page);

    match cmd {
        Cmd::List { limit } => {
            let mut list = all;
            let total = list.len();
            list.truncate(*limit as usize);
            if total > list.len() && !ctx.common.quiet {
                eprintln!(
                    "showing {} of {total} notifications — raise --limit to see the rest",
                    list.len()
                );
            }
            if list.is_empty() {
                note_empty(ctx, "notifications");
            }
            // The body is long prose; it belongs in `get`, not a list row.
            let rows: Vec<Value> = list
                .into_iter()
                .map(|mut n| {
                    if let Value::Object(m) = &mut n {
                        m.remove("body");
                    }
                    n
                })
                .collect();
            emit(
                ctx,
                "notification-list",
                json!({ "notifications": rows }),
                |v| {
                    output::table(&table_view(
                        &items(v, "notifications"),
                        &["id", "date", "subject", "to"],
                    ));
                },
            );
            Ok(())
        }
        Cmd::Get { notification_id } => {
            let found = all
                .iter()
                .find(|n| n.get("id").and_then(Value::as_str) == Some(notification_id.as_str()))
                .ok_or_else(|| {
                    CliError::NotFound(format!(
                        "no notification with id {notification_id} — run `wabhoa notifications list`"
                    ))
                })?;
            emit(ctx, "notification", found.clone(), output::render);
            Ok(())
        }
    }
}
