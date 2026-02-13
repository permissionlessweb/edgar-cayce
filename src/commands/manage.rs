use crate::state::Context;
use poise::serenity_prelude as serenity;

/// Clear session (stateless PoC — acknowledgment only)
#[poise::command(slash_command, guild_only)]
pub async fn clear(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    ctx.say("Session cleared.").await?;
    Ok(())
}

/// Create a new conversation thread
#[poise::command(slash_command, guild_only)]
pub async fn thread(
    ctx: Context<'_>,
    #[description = "Thread name"] name: Option<String>,
) -> Result<(), anyhow::Error> {
    let thread_name =
        name.unwrap_or_else(|| format!("Edgar - {}", ctx.author().name));

    let thread = ctx
        .channel_id()
        .create_thread(
            ctx.http(),
            serenity::CreateThread::new(thread_name.clone())
                .kind(serenity::ChannelType::PublicThread),
        )
        .await?;

    ctx.say(format!("Created thread: <#{}>", thread.id))
        .await?;
    Ok(())
}
