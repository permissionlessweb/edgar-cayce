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
}

/// A search result excerpt from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocExcerpt {
    pub doc_id: DocId,
    pub offset: usize,
    pub content: String,
    pub match_count: usize,
}
