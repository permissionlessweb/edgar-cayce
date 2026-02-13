use crate::state::Context;

/// Configure RLM parameters (admin only)
#[poise::command(slash_command, guild_only)]
pub async fn config(
    ctx: Context<'_>,
    #[description = "max_iterations | min_code_executions | min_answer_len"] param: Option<String>,
    #[description = "New value"] value: Option<u32>,
) -> Result<(), anyhow::Error> {
    let user_id = ctx.author().id.get();
    if !ctx.data().is_admin(user_id) {
        ctx.say("This command is admin-only.").await?;
        return Ok(());
    }

    match (param.as_deref(), value) {
        // Show current config
        (None, _) => {
            let config = ctx.data().rlm_config.read().await;
            ctx.say(format!(
                "**RLM Configuration:**\n\
                 `max_iterations`: {}\n\
                 `min_code_executions`: {}\n\
                 `min_answer_len`: {}",
                config.max_iterations, config.min_code_executions, config.min_answer_len
            ))
            .await?;
        }
        // Set a parameter
        (Some(key), Some(val)) => {
            let mut config = ctx.data().rlm_config.write().await;
            match key {
                "max_iterations" => {
                    config.max_iterations = val;
                    ctx.say(format!("`max_iterations` set to {}", val)).await?;
                }
                "min_code_executions" => {
                    config.min_code_executions = val;
                    ctx.say(format!("`min_code_executions` set to {}", val))
                        .await?;
                }
                "min_answer_len" => {
                    config.min_answer_len = val as usize;
                    ctx.say(format!("`min_answer_len` set to {}", val)).await?;
                }
                _ => {
                    ctx.say(format!(
                        "Unknown param `{}`. Valid: `max_iterations`, `min_code_executions`, `min_answer_len`",
                        key
                    ))
                    .await?;
                }
            }
        }
        (Some(_), None) => {
            ctx.say("Provide both `param` and `value`. Example: `/edgar config max_iterations 20`")
                .await?;
        }
    }

    Ok(())
}
