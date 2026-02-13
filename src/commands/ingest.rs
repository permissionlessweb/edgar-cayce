use crate::docs::ingest as doc_ingest;
use crate::state::Context;
use tracing::info;

/// Ingest a document from a URL (GitHub repo or web page)
#[poise::command(slash_command, guild_only)]
pub async fn ingest(
    ctx: Context<'_>,
    #[description = "URL (GitHub repo or web page)"] url: String,
    #[description = "Topic label for this document"] label: String,
    #[description = "Type: documentation, code, minimal"]
    doc_type: Option<String>,
) -> Result<(), anyhow::Error> {
    ctx.defer().await?;

    info!(
        user = ctx.author().name,
        url,
        label,
        "Ingestion started"
    );

    let store = &ctx.data().store;
    let is_github = url.contains("github.com");

    let (doc_id, detail) = if is_github {
        let (id, file_count) = doc_ingest::ingest_github_repo(
            store,
            &url,
            &label,
            doc_type.as_deref(),
        )
        .await?;
        (id, format!("{} files", file_count))
    } else {
        let (id, size) = doc_ingest::ingest_url(store, &url, &label).await?;
        (id, format!("{} bytes", size))
    };

    let meta = store.get_meta(&doc_id).await?;

    ctx.say(format!(
        "Ingested **{}** ({}) under topic **'{}'**\nDoc ID: `{}`\nSize: {} bytes",
        meta.name, detail, label, doc_id, meta.size
    ))
    .await?;

    Ok(())
}
