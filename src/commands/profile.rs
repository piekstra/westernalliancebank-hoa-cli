//! `wabhoa profile` — the account holder the portal has on file.

use pk_cli_core::{output, CliError};

use super::{emit, Ctx, PROFILE_PAGE};
use crate::parse;

pub fn run(ctx: &Ctx) -> Result<(), CliError> {
    let profile = ctx.read(|c| Ok(parse::profile(&c.get_text(PROFILE_PAGE)?)))?;
    emit(ctx, "profile", profile, output::render);
    Ok(())
}
