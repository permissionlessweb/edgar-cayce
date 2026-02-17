use serde::{Deserialize, Serialize};

/// Content-addressed document ID (blake3 hex hash).
pub type DocId = String;

/// Document metadata stored alongside content in cnidarium.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocMeta {
    pub id: DocId,
    pub name: String,
    /// e.g. "github:owner/repo" or "url:https://..."
    pub source: String,
    /// Topic label for /edgar ask scoping
    pub label: String,
    pub size: usize,
    pub ingested_at: i64,
    /// Admin-provided URL attribution context for citation routing.
    /// e.g. "files in docs/ map to https://akash.network/docs"
    #[serde(default)]
    pub url_context: Option<String>,
}

/// A stored Q/A record for dataset curation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaRecord {
    pub id: String,
    pub topic: String,
    pub question: String,
    pub answer: String,
    pub cited_urls: Vec<String>,
    pub doc_ids: Vec<String>,
    pub evidence: Vec<String>,
    pub iterations: u32,
    pub timestamp: i64,
}

/// A search result excerpt from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocExcerpt {
    pub doc_id: DocId,
    pub offset: usize,
    pub content: String,
    pub match_count: usize,
}
