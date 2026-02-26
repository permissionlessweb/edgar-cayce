pub mod ingest;
pub mod types;

use std::path::Path;

use anyhow::{Context, Result};
use cnidarium::{StateDelta, StateWrite, Storage};
use futures::StreamExt;
use tracing::{debug, warn};

use types::{DocExcerpt, DocId, DocMeta, QaRecord};

// Key prefixes (no trailing slashes — cnidarium convention)
const CONTENT_PREFIX: &str = "doc/content";
const META_PREFIX: &str = "doc/meta";
const LABEL_PREFIX: &str = "doc/label";
const QA_PREFIX: &str = "qa";

fn content_key(id: &str) -> String {
    format!("{}/{}", CONTENT_PREFIX, id)
}
fn meta_key(id: &str) -> String {
    format!("{}/{}", META_PREFIX, id)
}
fn label_key(label: &str, id: &str) -> String {
    format!("{}/{}:{}", LABEL_PREFIX, label, id)
}
fn qa_key(topic: &str, id: &str) -> String {
    format!("{}/{}/{}", QA_PREFIX, topic, id)
}

pub struct DocumentStore {
    storage: Storage,
    /// Cache document content in memory after first read to avoid repeated cnidarium lookups.
    content_cache: tokio::sync::RwLock<std::collections::HashMap<String, Vec<u8>>>,
}

impl DocumentStore {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let prefixes = vec![
            CONTENT_PREFIX.to_string(),
            META_PREFIX.to_string(),
            LABEL_PREFIX.to_string(),
            QA_PREFIX.to_string(),
        ];
        let storage = Storage::load(data_dir.to_path_buf(), prefixes)
            .await
            .context("Failed to init cnidarium storage")?;
        Ok(Self {
            storage,
            content_cache: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        })
    }

    /// Store a document. Returns its content-addressed DocId.
    /// Idempotent: same content = same ID.
    pub async fn store(
        &self,
        content: &[u8],
        name: &str,
        source: &str,
        label: &str,
        url_context: Option<&str>,
    ) -> Result<DocId> {
        let id = blake3::hash(content).to_hex().to_string();

        let meta = DocMeta {
            id: id.clone(),
            name: name.to_string(),
            source: source.to_string(),
            label: label.to_string(),
            size: content.len(),
            ingested_at: chrono::Utc::now().timestamp(),
            url_context: url_context.map(|s| s.to_string()),
        };

        let snapshot = self.storage.latest_snapshot();
        let mut delta = StateDelta::new(snapshot);

        delta.put_raw(content_key(&id), content.to_vec());
        delta.put_raw(
            meta_key(&id),
            serde_json::to_vec(&meta).context("serialize meta")?,
        );
        // Label index entry (empty value — presence is the index)
        delta.put_raw(label_key(label, &id), vec![]);

        self.storage.commit(delta).await?;
        debug!(doc_id = %id, name, label, size = content.len(), "document stored");
        Ok(id)
    }

    pub async fn get_content(&self, doc_id: &str) -> Result<Vec<u8>> {
        // Check cache first
        {
            let cache = self.content_cache.read().await;
            if let Some(content) = cache.get(doc_id) {
                return Ok(content.clone());
            }
        }

        let snapshot = self.storage.latest_snapshot();
        use cnidarium::StateRead;
        let content = snapshot
            .get_raw(&content_key(doc_id))
            .await?
            .ok_or_else(|| anyhow::anyhow!("document not found: {}", doc_id))?;

        // Cache for subsequent reads
        {
            let mut cache = self.content_cache.write().await;
            cache.insert(doc_id.to_string(), content.clone());
        }

        Ok(content)
    }

    pub async fn get_meta(&self, doc_id: &str) -> Result<DocMeta> {
        let snapshot = self.storage.latest_snapshot();
        use cnidarium::StateRead;
        let bytes = snapshot
            .get_raw(&meta_key(doc_id))
            .await?
            .ok_or_else(|| anyhow::anyhow!("document metadata not found: {}", doc_id))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// List all documents, newest first.
    pub async fn list(&self, limit: usize, _offset: usize) -> Result<Vec<DocMeta>> {
        let snapshot = self.storage.latest_snapshot();
        use cnidarium::StateRead;
        let mut stream = snapshot.prefix_raw(META_PREFIX);
        let mut results = Vec::new();

        while let Some(entry) = stream.next().await {
            match entry {
                Ok((_key, value)) => {
                    if let Ok(meta) = serde_json::from_slice::<DocMeta>(&value) {
                        results.push(meta);
                    }
                }
                Err(e) => {
                    warn!("Error reading doc meta stream: {}", e);
                }
            }
        }

        // Sort by ingested_at descending
        results.sort_by(|a, b| b.ingested_at.cmp(&a.ingested_at));
        results.truncate(limit);
        Ok(results)
    }

    /// List documents with a specific label.
    pub async fn list_by_label(&self, label: &str) -> Result<Vec<DocMeta>> {
        let snapshot = self.storage.latest_snapshot();
        use cnidarium::StateRead;
        let prefix = format!("{}/{}:", LABEL_PREFIX, label);
        let mut stream = snapshot.prefix_raw(&prefix);
        let mut results = Vec::new();

        while let Some(entry) = stream.next().await {
            match entry {
                Ok((key, _)) => {
                    // Key format: "doc/label/{label}:{doc_id}"
                    let key_str = String::from_utf8_lossy(key.as_bytes());
                    if let Some(doc_id) = key_str.strip_prefix(&prefix) {
                        match self.get_meta(doc_id).await {
                            Ok(meta) => results.push(meta),
                            Err(e) => warn!("Failed to get meta for {}: {}", doc_id, e),
                        }
                    }
                }
                Err(e) => {
                    warn!("Error reading label index: {}", e);
                }
            }
        }

        Ok(results)
    }

    /// Delete a document and its label index.
    pub async fn delete(&self, doc_id: &str) -> Result<()> {
        // Get meta first for label cleanup
        let meta = self.get_meta(doc_id).await?;
        let snapshot = self.storage.latest_snapshot();
        let mut delta = StateDelta::new(snapshot);
        delta.delete(content_key(doc_id));
        delta.delete(meta_key(doc_id));
        delta.delete(label_key(&meta.label, doc_id));
        self.storage.commit(delta).await?;
        debug!(doc_id, "document deleted");
        Ok(())
    }

    /// Read a char-range section from a document. Capped at 100K chars.
    pub async fn get_section(
        &self,
        doc_id: &str,
        offset: usize,
        length: usize,
    ) -> Result<String> {
        let content = self.get_content(doc_id).await?;
        let text = String::from_utf8_lossy(&content);
        let chars: Vec<char> = text.chars().collect();
        let len = length.min(100_000);
        let start = offset.min(chars.len());
        let end = (start + len).min(chars.len());
        Ok(chars[start..end].iter().collect())
    }

    /// Keyword search within a document. Splits query into words, matches ANY word (OR logic).
    /// Returns excerpts with context window around each match.
    pub async fn search(
        &self,
        doc_id: &str,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<DocExcerpt>> {
        let content = self.get_content(doc_id).await?;
        let text = String::from_utf8_lossy(&content);
        let text_lower = text.to_lowercase();

        // Split query into individual keywords for OR matching
        let keywords: Vec<String> = query
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() >= 2)
            .collect();

        if keywords.is_empty() {
            return Ok(vec![]);
        }

        let context_window = 300; // chars of context around match
        let mut results = Vec::new();
        let mut seen_offsets = std::collections::HashSet::new();

        for keyword in &keywords {
            let mut search_from = 0;
            while results.len() < max_results * 2 {
                let Some(byte_pos) = text_lower[search_from..].find(keyword.as_str()) else {
                    break;
                };
                let abs_byte_pos = search_from + byte_pos;
                let char_pos = text_lower[..abs_byte_pos].chars().count();

                // Skip if we already have a match near this offset
                let nearby = seen_offsets.iter().any(|&o: &usize| char_pos.abs_diff(o) < context_window);
                if !nearby {
                    seen_offsets.insert(char_pos);
                    let chars: Vec<char> = text.chars().collect();
                    let start = char_pos.saturating_sub(context_window);
                    let end = (char_pos + keyword.len() + context_window).min(chars.len());
                    let excerpt: String = chars[start..end].iter().collect();

                    // Count how many keywords appear in this excerpt
                    let excerpt_lower = excerpt.to_lowercase();
                    let match_count = keywords.iter().filter(|k| excerpt_lower.contains(k.as_str())).count();

                    results.push(DocExcerpt {
                        doc_id: doc_id.to_string(),
                        offset: char_pos,
                        content: excerpt,
                        match_count,
                    });
                }

                search_from = abs_byte_pos + keyword.len().max(1);
                if search_from >= text_lower.len() {
                    break;
                }
            }
        }

        // Sort by match_count descending (excerpts matching more keywords first)
        results.sort_by(|a, b| b.match_count.cmp(&a.match_count));
        results.truncate(max_results);
        Ok(results)
    }

    /// Extract file/section headers from an ingested document.
    /// Githem-core uses `=== filename ===` as section delimiters.
    pub async fn list_files(&self, doc_id: &str) -> Result<Vec<(usize, String)>> {
        let content = self.get_content(doc_id).await?;
        let text = String::from_utf8_lossy(&content);
        let mut files = Vec::new();
        let mut char_offset = 0;

        for line in text.lines() {
            if line.starts_with("=== ") && line.ends_with(" ===") {
                let name = line.trim_start_matches("=== ").trim_end_matches(" ===");
                files.push((char_offset, name.to_string()));
            }
            char_offset += line.chars().count() + 1; // +1 for newline
        }

        Ok(files)
    }

    /// Get unique labels for autocomplete.
    pub async fn labels(&self) -> Result<Vec<String>> {
        let snapshot = self.storage.latest_snapshot();
        use cnidarium::StateRead;
        let mut stream = snapshot.prefix_raw(LABEL_PREFIX);
        let mut labels = std::collections::BTreeSet::new();

        while let Some(entry) = stream.next().await {
            if let Ok((key, _)) = entry {
                let key_str = String::from_utf8_lossy(key.as_bytes());
                // Key format: "doc/label/{label}:{doc_id}"
                if let Some(rest) = key_str.strip_prefix(&format!("{}/", LABEL_PREFIX)) {
                    if let Some(label) = rest.split(':').next() {
                        labels.insert(label.to_string());
                    }
                }
            }
        }

        Ok(labels.into_iter().collect())
    }

    /// Store a Q/A record for dataset curation.
    pub async fn store_qa(&self, record: &QaRecord) -> Result<()> {
        let snapshot = self.storage.latest_snapshot();
        let mut delta = StateDelta::new(snapshot);
        delta.put_raw(
            qa_key(&record.topic, &record.id),
            serde_json::to_vec(record).context("serialize QaRecord")?,
        );
        self.storage.commit(delta).await?;
        debug!(qa_id = %record.id, topic = %record.topic, "Q/A record stored");
        Ok(())
    }

    /// List Q/A records for a topic, newest first.
    pub async fn list_qa(&self, topic: &str, limit: usize) -> Result<Vec<QaRecord>> {
        let snapshot = self.storage.latest_snapshot();
        use cnidarium::StateRead;
        let prefix = format!("{}/{}/", QA_PREFIX, topic);
        let mut stream = snapshot.prefix_raw(&prefix);
        let mut results = Vec::new();

        while let Some(entry) = stream.next().await {
            match entry {
                Ok((_key, value)) => {
                    if let Ok(record) = serde_json::from_slice::<QaRecord>(&value) {
                        results.push(record);
                    }
                }
                Err(e) => {
                    warn!("Error reading QA stream: {}", e);
                }
            }
        }

        results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        results.truncate(limit);
        Ok(results)
    }
}
