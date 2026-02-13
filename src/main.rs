mod commands;
mod docs;
mod llm;
mod rlm;
mod state;

use std::collections::HashSet;
use std::sync::Arc;

use poise::serenity_prelude as serenity;
use poise::{Framework, FrameworkOptions};
use tokio::sync::RwLock;
use tracing::{error, info, Level};

use docs::DocumentStore;
use llm::LlmClient;
use rlm::RlmEngine;
use state::{AppState, RlmConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    // Load env
    let _ = dotenv::dotenv();
    let token = dotenv::var("DISCORD_TOKEN").expect("DISCORD_TOKEN required");
    let guild_id: Option<serenity::GuildId> = dotenv::var("DISCORD_GUILD_ID")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(serenity::GuildId::new);

    // Init storage
    let data_dir = std::path::PathBuf::from("./data/docs");
    let store = Arc::new(DocumentStore::new(&data_dir).await?);
    info!("Document store initialized at {:?}", data_dir);

    // Init LLM client
    let llm_client = Arc::new(LlmClient::from_env()?);
    info!("LLM client initialized");

    // Parse admin user IDs from env
    let admin_ids: HashSet<u64> = dotenv::var("ADMIN_USER_IDS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse::<u64>().ok())
        .collect();
    if !admin_ids.is_empty() {
        info!(count = admin_ids.len(), "Admin users configured");
    }

    let rlm_config = Arc::new(RwLock::new(RlmConfig::default()));

    // Init RLM engine
    let rlm = Arc::new(RlmEngine::new(llm_client.clone(), store.clone()));

    let app_state = AppState {
        store,
        llm: llm_client,
        rlm,
        admin_ids,
        rlm_config,
    };

    let intents =
        serenity::GatewayIntents::GUILDS | serenity::GatewayIntents::GUILD_MESSAGES;

    let framework = Framework::builder()
        .options(FrameworkOptions {
            commands: vec![commands::edgar()],
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                info!("Bot connected as: {} ({})", ready.user.name, ready.user.id);

                let commands = &framework.options().commands;
                info!("Registering {} top-level command(s):", commands.len());
                for cmd in commands {
                    info!("  /{} ({} subcommands)", cmd.name, cmd.subcommands.len());
                    for sub in &cmd.subcommands {
                        info!("    /{} {}", cmd.name, sub.name);
                    }
                }

                if let Some(gid) = guild_id {
                    info!("Registering to guild {} (instant)", gid);
                    poise::builtins::register_in_guild(
                        ctx,
                        &framework.options().commands,
                        gid,
                    )
                    .await?;
                } else {
                    info!("Registering globally (up to 1 hour delay)");
                    poise::builtins::register_globally(
                        ctx,
                        &framework.options().commands,
                    )
                    .await?;
                }

                Ok(app_state)
            })
        })
        .build();

    info!("Starting Edgar Discord bot...");

    let mut client = serenity::ClientBuilder::new(&token, intents)
        .framework(framework)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create client: {}", e))?;

    if let Err(e) = client.start().await {
        error!("Client error: {}", e);
    }

    Ok(())
}
