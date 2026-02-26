use poise::serenity_prelude as serenity;

use crate::state::Context;

/// Configure Edgar bot settings (admin only)
#[poise::command(
    slash_command,
    guild_only,
    subcommands("rlm", "roles_list", "roles_add", "roles_remove")
)]
pub async fn config(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

/// View or set RLM reasoning parameters
#[poise::command(slash_command, guild_only)]
pub async fn rlm(
    ctx: Context<'_>,
    #[description = "Max reasoning iterations"] max_iterations: Option<u32>,
    #[description = "Min code executions required"] min_code_executions: Option<u32>,
    #[description = "Min answer length (chars)"] min_answer_len: Option<u32>,
    #[description = "Parallel reasoning loops"] parallel_loops: Option<u32>,
) -> Result<(), anyhow::Error> {
    if !is_admin(&ctx).await {
        ctx.say("This command is admin-only.").await?;
        return Ok(());
    }

    let has_updates = max_iterations.is_some()
        || min_code_executions.is_some()
        || min_answer_len.is_some()
        || parallel_loops.is_some();

    if has_updates {
        let mut config = ctx.data().rlm_config.write().await;
        let mut changes = Vec::new();

        if let Some(v) = max_iterations {
            config.max_iterations = v;
            changes.push(format!("`max_iterations` -> {v}"));
        }
        if let Some(v) = min_code_executions {
            config.min_code_executions = v;
            changes.push(format!("`min_code_executions` -> {v}"));
        }
        if let Some(v) = min_answer_len {
            config.min_answer_len = v as usize;
            changes.push(format!("`min_answer_len` -> {v}"));
        }
        if let Some(v) = parallel_loops {
            config.parallel_loops = v;
            changes.push(format!("`parallel_loops` -> {v}"));
        }

        ctx.say(format!("**Updated:**\n{}", changes.join("\n")))
            .await?;
    } else {
        let config = ctx.data().rlm_config.read().await;
        ctx.say(format!(
            "**RLM Configuration:**\n\
             `max_iterations`: {}\n\
             `min_code_executions`: {}\n\
             `min_answer_len`: {}\n\
             `parallel_loops`: {}",
            config.max_iterations,
            config.min_code_executions,
            config.min_answer_len,
            config.parallel_loops,
        ))
        .await?;
    }

    Ok(())
}

/// List configured admin roles
#[poise::command(slash_command, guild_only, rename = "roles-list")]
pub async fn roles_list(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    if !is_admin(&ctx).await {
        ctx.say("This command is admin-only.").await?;
        return Ok(());
    }

    let role_ids = ctx.data().admin_role_ids.read().await;
    if role_ids.is_empty() {
        ctx.say("**Admin Roles:** none configured").await?;
    } else {
        let list: Vec<String> = role_ids.iter().map(|id| format!("<@&{id}>")).collect();
        ctx.say(format!("**Admin Roles:**\n{}", list.join("\n")))
            .await?;
    }

    Ok(())
}

/// Add a Discord role as an admin role
#[poise::command(slash_command, guild_only, rename = "roles-add")]
pub async fn roles_add(
    ctx: Context<'_>,
    #[description = "Role to grant admin access"] role: serenity::Role,
) -> Result<(), anyhow::Error> {
    if !is_admin(&ctx).await {
        ctx.say("This command is admin-only.").await?;
        return Ok(());
    }

    let role_id = role.id.get();
    let inserted = ctx.data().admin_role_ids.write().await.insert(role_id);

    if inserted {
        ctx.say(format!("Added <@&{role_id}> as admin role."))
            .await?;
    } else {
        ctx.say(format!("<@&{role_id}> is already an admin role."))
            .await?;
    }

    Ok(())
}

/// Remove a Discord role from admin roles
#[poise::command(slash_command, guild_only, rename = "roles-remove")]
pub async fn roles_remove(
    ctx: Context<'_>,
    #[description = "Role to revoke admin access"] role: serenity::Role,
) -> Result<(), anyhow::Error> {
    if !is_admin(&ctx).await {
        ctx.say("This command is admin-only.").await?;
        return Ok(());
    }

    let role_id = role.id.get();
    let removed = ctx.data().admin_role_ids.write().await.remove(&role_id);

    if removed {
        ctx.say(format!("Removed <@&{role_id}> from admin roles."))
            .await?;
    } else {
        ctx.say(format!("<@&{role_id}> was not an admin role."))
            .await?;
    }

    Ok(())
}

/// Check if the invoking user is an admin via: user ID allowlist → guild owner → admin roles.
pub async fn is_admin(ctx: &Context<'_>) -> bool {
    let user_id = ctx.author().id.get();

    // 1. Explicit user ID allowlist (env var)
    if ctx.data().admin_ids.contains(&user_id) {
        return true;
    }

    // 2. Guild owner (uses cache, no API call)
    if let Some(guild) = ctx.guild() {
        if guild.owner_id.get() == user_id {
            return true;
        }
    }

    // 3. Admin roles (if any configured)
    let role_ids = ctx.data().admin_role_ids.read().await;
    if !role_ids.is_empty() {
        if let Some(member) = ctx.author_member().await {
            if member.roles.iter().any(|r| role_ids.contains(&r.get())) {
                return true;
            }
        }
    }

    false
}
