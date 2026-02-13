use anyhow::{Context, Result};
use tracing::info;

use super::types::DocId;
use super::DocumentStore;

/// Ingest a GitHub repository using githem-core.
/// Returns (doc_id, file_count).
pub async fn ingest_github_repo(
    store: &DocumentStore,
    url: &str,
    label: &str,
    doc_type: Option<&str>,
) -> Result<(DocId, usize)> {
    // Validate GitHub URL
    let _parsed =
        githem_core::parse_github_url(url).context("Invalid GitHub URL")?;

    let preset = match doc_type {
        Some("code") => githem_core::FilterPreset::CodeOnly,
        Some("minimal") => githem_core::FilterPreset::Minimal,
        _ => githem_core::FilterPreset::Standard,
    };

    let opts = githem_core::IngestOptions::with_preset(preset);

    // Clone and ingest — this is blocking I/O so run in spawn_blocking
    let url_owned = url.to_string();
    let output = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let ingester =
            githem_core::Ingester::from_url_cached(&url_owned, opts)?;
        let mut output = Vec::new();
        ingester.ingest(&mut output)?;
        Ok(output)
    })
    .await
    .context("spawn_blocking join failed")??;

    // Count files from githem output format: "=== path/to/file ===\n"
    let text = String::from_utf8_lossy(&output);
    let file_count = text.matches("=== ").count();

    // Extract repo name from URL for the document name
    let name = url
        .trim_end_matches('/')
        .rsplit('/')
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("/");

    let source = format!("github:{}", name);
    let doc_id = store.store(&output, &name, &source, label).await?;

    info!(
        doc_id = %doc_id,
        file_count,
        size = output.len(),
        label,
        "GitHub repo ingested"
    );

    Ok((doc_id, file_count))
}

/// Ingest a web page by fetching its content.
pub async fn ingest_url(
    store: &DocumentStore,
    url: &str,
    label: &str,
) -> Result<(DocId, usize)> {
    let resp = reqwest::get(url)
        .await
        .context("Failed to fetch URL")?;

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body = resp.bytes().await.context("Failed to read response body")?;

    // Convert HTML to text if applicable
    let text = if content_type.contains("html") {
        html2text::from_read(&body[..], 120)
            .unwrap_or_else(|_| String::from_utf8_lossy(&body).to_string())
    } else {
        String::from_utf8_lossy(&body).to_string()
    };

    let name = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(url);
    let source = format!("url:{}", url);
    let doc_id = store.store(text.as_bytes(), name, &source, label).await?;

    info!(doc_id = %doc_id, size = text.len(), label, "URL ingested");
    Ok((doc_id, text.len()))
}
