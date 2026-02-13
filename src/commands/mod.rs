mod ask;
mod config;
mod ingest;
mod manage;
mod sources;

use crate::state::Context;

/// Edgar - Ergors Discord Knowledge Assistant
#[poise::command(
    slash_command,
    subcommands(
        "ask::ask",
        "ingest::ingest",
        "sources::sources",
        "manage::clear",
        "manage::thread",
        "config::config"
    )
)]
pub async fn edgar(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}
