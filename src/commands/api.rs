//! `wabhoa api` — raw passthrough to any portal endpoint.
//!
//! The escape hatch for surfaces the typed commands don't cover, and the
//! quickest way to check whether the portal's responses have drifted.
//!
//! POST is supported because several of the portal's *reads* are POSTs (its
//! search filters are JSON bodies, not query strings) — but a POST to any
//! endpoint in [`crate::writes`] is refused, so the escape hatch cannot move
//! money while this CLI advertises itself as read-only.

use clap::Args;
use pk_cli_core::{output, CliError};
use serde_json::Value;

use super::Ctx;
use crate::writes;

#[derive(Args, Debug)]
pub struct ApiArgs {
    /// Path under the portal host, e.g. `/DashboardContent`.
    pub path: String,

    /// JSON body to POST. Without it the request is a GET.
    #[arg(long, value_name = "JSON")]
    pub data: Option<String>,

    /// Print the raw response body instead of parsing it as JSON. Most portal
    /// paths answer with HTML, so this is how you inspect them.
    #[arg(long)]
    pub raw: bool,
}

pub fn run(ctx: &Ctx, args: &ApiArgs) -> Result<(), CliError> {
    // Validate everything before opening a session or touching the network.
    let body = match &args.data {
        Some(raw) => Some(
            serde_json::from_str::<Value>(raw)
                .map_err(|e| CliError::Usage(format!("--data must be valid JSON: {e}")))?,
        ),
        None => None,
    };

    if body.is_some() && writes::is_write(&args.path) {
        return Err(CliError::ConfirmationRequired(format!(
            "{} is a write endpoint and this CLI is read-only — see `wabhoa writes`",
            args.path
        )));
    }

    let client = ctx.client()?;
    match body {
        Some(body) => {
            let payload = client.post_json(&args.path, &body)?;
            output::json(&payload);
        }
        None if args.raw => print!("{}", client.get_text(&args.path)?),
        None => {
            let text = client.get_text(&args.path)?;
            match serde_json::from_str::<Value>(&text) {
                Ok(payload) => output::json(&payload),
                Err(_) => {
                    return Err(CliError::Usage(format!(
                        "{} did not answer with JSON — re-run with --raw to see the body",
                        args.path
                    )))
                }
            }
        }
    }
    Ok(())
}
