use std::collections::HashSet;

use crate::docs::types::DocMeta;

/// A parsed URL template extracted from a doc's `url_context`.
#[derive(Debug, Clone)]
pub struct UrlTemplate {
    /// The URL prefix before the `{filepath}` placeholder.
    pub prefix: String,
    /// The URL suffix after the `{filepath}` placeholder (usually empty).
    pub suffix: String,
}

impl UrlTemplate {
    /// Resolve a filepath into a full URL using this template.
    pub fn resolve(&self, filepath: &str) -> String {
        format!("{}{}{}", self.prefix, filepath, self.suffix)
    }
}

/// Parse a `url_context` string into a `UrlTemplate`.
///
/// Supports two patterns:
/// 1. Template with `{filepath}` placeholder — splits around it.
///    e.g. `"Source files ... at https://github.com/owner/repo/blob/main/{filepath}"`
/// 2. Plain URL (no placeholder) — uses as base, appends `/` + filepath.
///
/// Returns `None` if no URL can be extracted.
pub fn parse_url_template(url_context: &str) -> Option<UrlTemplate> {
    // Check for {filepath} placeholder first
    if let Some(pos) = url_context.find("{filepath}") {
        // Extract the URL portion leading up to {filepath}
        let before = &url_context[..pos];
        let after = &url_context[pos + "{filepath}".len()..];

        // Find the URL start (https://) in the text before the placeholder
        let url_start = before.rfind("https://").or_else(|| before.rfind("http://"))?;
        let prefix = &before[url_start..];

        // Suffix: take any URL-like chars after the placeholder, stop at whitespace/end
        let suffix: String = after.chars().take_while(|c| !c.is_whitespace()).collect();

        Some(UrlTemplate {
            prefix: prefix.to_string(),
            suffix,
        })
    } else {
        // No placeholder — extract any URL and use as base
        extract_base_url(url_context).map(|base| {
            let base = base.trim_end_matches('/');
            UrlTemplate {
                prefix: format!("{}/", base),
                suffix: String::new(),
            }
        })
    }
}

/// Extract the first HTTP(S) URL from a string.
fn extract_base_url(text: &str) -> Option<String> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let url_part = &text[start..];
    // Take until whitespace or end
    let end = url_part
        .find(|c: char| c.is_whitespace())
        .unwrap_or(url_part.len());
    let url = &url_part[..end];
    // Strip trailing punctuation that's likely not part of the URL
    let url = url.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ')' | ']'));
    if url.len() > 10 {
        Some(url.to_string())
    } else {
        None
    }
}

/// Resolve accessed files into citation URLs, deduplicating against URLs the LLM already produced.
///
/// - `accessed_files`: `(doc_id, matched_filename)` pairs from the REPL tracker
/// - `topic_docs`: the docs loaded for this topic (to look up `url_context`)
/// - `existing_urls`: URLs the LLM already included in its answer
///
/// Returns new URLs to add (not already in `existing_urls`).
pub fn resolve_citations(
    accessed_files: &[(String, String)],
    topic_docs: &[DocMeta],
    existing_urls: &[String],
) -> Vec<String> {
    let existing: HashSet<&str> = existing_urls.iter().map(|u| u.as_str()).collect();
    let mut new_urls = Vec::new();
    let mut seen = HashSet::new();

    for (doc_id, filename) in accessed_files {
        // Find the doc's url_context
        let Some(doc) = topic_docs.iter().find(|d| d.id == *doc_id) else {
            continue;
        };
        let Some(url_context) = &doc.url_context else {
            continue;
        };
        let Some(template) = parse_url_template(url_context) else {
            continue;
        };

        let url = template.resolve(filename);

        // Dedup: skip if LLM already cited this URL or we already added it
        if existing.contains(url.as_str()) || !seen.insert(url.clone()) {
            continue;
        }

        // Also skip if the existing URLs contain this URL as a substring (partial match)
        if existing.iter().any(|e| e.contains(&url) || url.contains(*e)) {
            continue;
        }

        new_urls.push(url);
    }

    new_urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_template() {
        let ctx = "Source files from this repository are publicly viewable at https://github.com/akash-network/provider/blob/main/{filepath}";
        let tmpl = parse_url_template(ctx).unwrap();
        assert_eq!(
            tmpl.prefix,
            "https://github.com/akash-network/provider/blob/main/"
        );
        assert_eq!(tmpl.suffix, "");
        assert_eq!(
            tmpl.resolve("cmd/provider-services/main.go"),
            "https://github.com/akash-network/provider/blob/main/cmd/provider-services/main.go"
        );
    }

    #[test]
    fn test_parse_template_with_suffix() {
        let ctx = "https://example.com/docs/{filepath}#latest";
        let tmpl = parse_url_template(ctx).unwrap();
        assert_eq!(tmpl.prefix, "https://example.com/docs/");
        assert_eq!(tmpl.suffix, "#latest");
        assert_eq!(
            tmpl.resolve("guide.md"),
            "https://example.com/docs/guide.md#latest"
        );
    }

    #[test]
    fn test_parse_plain_url_no_placeholder() {
        let ctx = "files in docs/ map to https://akash.network/docs";
        let tmpl = parse_url_template(ctx).unwrap();
        assert_eq!(tmpl.prefix, "https://akash.network/docs/");
        assert_eq!(tmpl.suffix, "");
        assert_eq!(
            tmpl.resolve("getting-started.md"),
            "https://akash.network/docs/getting-started.md"
        );
    }

    #[test]
    fn test_parse_no_url() {
        assert!(parse_url_template("no url here").is_none());
    }

    #[test]
    fn test_resolve_citations_basic() {
        let docs = vec![DocMeta {
            id: "abc123".to_string(),
            name: "test-repo".to_string(),
            source: "github:owner/repo".to_string(),
            label: "test".to_string(),
            size: 1000,
            ingested_at: 0,
            url_context: Some(
                "https://github.com/owner/repo/blob/main/{filepath}".to_string(),
            ),
        }];

        let accessed = vec![
            ("abc123".to_string(), "src/main.rs".to_string()),
            ("abc123".to_string(), "README.md".to_string()),
        ];

        let existing: Vec<String> = vec![];
        let new_urls = resolve_citations(&accessed, &docs, &existing);
        assert_eq!(new_urls.len(), 2);
        assert_eq!(
            new_urls[0],
            "https://github.com/owner/repo/blob/main/src/main.rs"
        );
        assert_eq!(
            new_urls[1],
            "https://github.com/owner/repo/blob/main/README.md"
        );
    }

    #[test]
    fn test_resolve_citations_dedup_existing() {
        let docs = vec![DocMeta {
            id: "abc123".to_string(),
            name: "test-repo".to_string(),
            source: "github:owner/repo".to_string(),
            label: "test".to_string(),
            size: 1000,
            ingested_at: 0,
            url_context: Some(
                "https://github.com/owner/repo/blob/main/{filepath}".to_string(),
            ),
        }];

        let accessed = vec![
            ("abc123".to_string(), "src/main.rs".to_string()),
            ("abc123".to_string(), "README.md".to_string()),
        ];

        // LLM already cited src/main.rs
        let existing = vec![
            "https://github.com/owner/repo/blob/main/src/main.rs".to_string(),
        ];
        let new_urls = resolve_citations(&accessed, &docs, &existing);
        assert_eq!(new_urls.len(), 1);
        assert_eq!(
            new_urls[0],
            "https://github.com/owner/repo/blob/main/README.md"
        );
    }

    #[test]
    fn test_resolve_citations_dedup_self() {
        let docs = vec![DocMeta {
            id: "abc123".to_string(),
            name: "test-repo".to_string(),
            source: "github:owner/repo".to_string(),
            label: "test".to_string(),
            size: 1000,
            ingested_at: 0,
            url_context: Some(
                "https://github.com/owner/repo/blob/main/{filepath}".to_string(),
            ),
        }];

        // Same file accessed twice
        let accessed = vec![
            ("abc123".to_string(), "src/main.rs".to_string()),
            ("abc123".to_string(), "src/main.rs".to_string()),
        ];

        let new_urls = resolve_citations(&accessed, &docs, &[]);
        assert_eq!(new_urls.len(), 1);
    }

    #[test]
    fn test_resolve_citations_no_url_context() {
        let docs = vec![DocMeta {
            id: "abc123".to_string(),
            name: "test-repo".to_string(),
            source: "github:owner/repo".to_string(),
            label: "test".to_string(),
            size: 1000,
            ingested_at: 0,
            url_context: None,
        }];

        let accessed = vec![("abc123".to_string(), "src/main.rs".to_string())];
        let new_urls = resolve_citations(&accessed, &docs, &[]);
        assert!(new_urls.is_empty());
    }

    #[test]
    fn test_resolve_citations_unknown_doc() {
        let docs = vec![DocMeta {
            id: "abc123".to_string(),
            name: "test-repo".to_string(),
            source: "github:owner/repo".to_string(),
            label: "test".to_string(),
            size: 1000,
            ingested_at: 0,
            url_context: Some(
                "https://github.com/owner/repo/blob/main/{filepath}".to_string(),
            ),
        }];

        // doc_id doesn't match any doc
        let accessed = vec![("unknown_id".to_string(), "src/main.rs".to_string())];
        let new_urls = resolve_citations(&accessed, &docs, &[]);
        assert!(new_urls.is_empty());
    }
}
