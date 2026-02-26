use crate::commands::config::is_admin;
use crate::state::Context;
use tracing::info;

/// Ask a question about ingested documents
#[poise::command(slash_command, guild_only)]
pub async fn ask(
    ctx: Context<'_>,
    #[description = "Topic (matches ingested label)"]
    #[autocomplete = "autocomplete_topic"]
    topic: String,
    #[description = "Your question"] question: String,
    #[description = "Show debug evidence (admin only)"] debug: Option<bool>,
) -> Result<(), anyhow::Error> {
    // Acknowledge immediately so the user isn't staring at a loading spinner
    let user_mention = format!("<@{}>", ctx.author().id);
    ctx.say(format!(
        "Got it — researching **{}** for you. I'll ping you when the answer is ready, {}",
        topic, user_mention
    ))
    .await?;

    let is_admin = is_admin(&ctx).await;
    let show_debug = debug.unwrap_or(false) && is_admin;

    // Read current config
    let config = ctx.data().rlm_config.read().await;
    let max_iterations = config.max_iterations;
    let min_code_executions = config.min_code_executions;
    let min_answer_len = config.min_answer_len;
    let parallel_loops = config.parallel_loops;
    drop(config);

    info!(
        user = ctx.author().name,
        topic, question, is_admin, "RLM query started"
    );

    let result = ctx
        .data()
        .rlm
        .query(
            &topic,
            &question,
            max_iterations,
            min_code_executions,
            min_answer_len,
            parallel_loops,
        )
        .await?;

    info!(
        iterations = result.iterations,
        answer_len = result.answer.len(),
        evidence_count = result.evidence.len(),
        "RLM query complete"
    );

    let mut full = format!(
        "{} here's what I found:\n\n**Q:** {}\n**Topic:** {} | **Iterations:** {}\n\n**A:** {}",
        user_mention, question, topic, result.iterations, result.answer
    );

    // Append cited URLs as clickable Discord markdown links
    if !result.cited_urls.is_empty() {
        full.push_str("\n\n**Sources:**\n");
        for url in &result.cited_urls {
            // Extract a short label from the URL path
            let label = url.rsplit('/').find(|s| !s.is_empty()).unwrap_or(url);
            full.push_str(&format!("- [{}]({})\n", label, url));
        }
    }

    // Admin-only debug evidence
    if show_debug && !result.evidence.is_empty() {
        full.push_str("\n\n---\n**[Debug] Evidence collected from documents:**\n");
        for (i, ev) in result.evidence.iter().enumerate().take(3) {
            let snippet = if ev.len() > 800 {
                &ev[..800]
            } else {
                ev.as_str()
            };
            full.push_str(&format!("\n**[{}]** ```\n{}```\n", i + 1, snippet));
        }
    }

    // Send in chunks if needed
    send_chunked(&ctx, &full).await
}

/// Send a message in Discord-safe chunks (max 1990 chars).
/// Uses ctx.say() for all chunks — poise routes follow-ups through the
/// interaction webhook, which doesn't require Send Messages channel permission.
async fn send_chunked(ctx: &Context<'_>, text: &str) -> Result<(), anyhow::Error> {
    let mut remaining = text;
    while !remaining.is_empty() {
        let chunk_len = remaining.len().min(1990);
        let split_at = if chunk_len < remaining.len() {
            remaining[..chunk_len]
                .rfind('\n')
                .or_else(|| remaining[..chunk_len].rfind(' '))
                .map(|i| i + 1)
                .unwrap_or(chunk_len)
        } else {
            chunk_len
        };
        let chunk = &remaining[..split_at];
        remaining = &remaining[split_at..];

        ctx.say(chunk).await?;
    }
    Ok(())
}

/// Autocomplete for topic names from ingested document labels.
async fn autocomplete_topic(ctx: Context<'_>, partial: &str) -> Vec<String> {
    let labels = ctx.data().store.labels().await.unwrap_or_default();

    labels
        .into_iter()
        .filter(|l| l.to_lowercase().contains(&partial.to_lowercase()))
        .take(25)
        .collect()
}
