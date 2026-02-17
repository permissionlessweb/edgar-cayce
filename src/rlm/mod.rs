pub mod exec;
pub mod prompts;
pub mod repl;

use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, info, warn};

use crate::docs::types::DocMeta;
use crate::docs::DocumentStore;
use crate::llm::{LlmClient, Message};

use exec::PersistentSession;
use repl::Command;

/// Patterns that indicate the LLM refused to engage or produced a non-answer.
const BROKEN_ANSWER_PATTERNS: &[&str] = &[
    "i don't have the ability",
    "i cannot access",
    "i apologize",
    "i'm unable to",
    "unable to directly",
    "i can't access",
    "don't have access",
    "cannot directly read",
    "limitations of this interface",
    "provide the content or specific sections",
    "if you provide the content",
    "do not contain specific details",
    "does not contain specific",
    "no mention of",
    "there is no mention",
    "the excerpts do not",
    "the provided document excerpts do not",
    "not contain content related",
];

const STOP_WORDS: &[&str] = &[
    "what", "which", "where", "when", "does", "have", "with", "that", "this", "from", "about",
    "some", "there", "their", "they", "your", "been", "were", "how", "could", "would", "should",
    "shall", "will", "into", "also", "just", "like", "make", "using", "used", "need", "want",
    "find", "know", "tell", "many", "much", "very", "really", "please", "help", "more", "most",
    "only",
];

pub struct RlmResponse {
    pub answer: String,
    pub iterations: u32,
    pub sources: Vec<String>,
    /// Raw evidence collected from REPL outputs (document content the LLM actually read)
    pub evidence: Vec<String>,
}

pub struct RlmEngine {
    llm: Arc<LlmClient>,
    store: Arc<DocumentStore>,
}

impl RlmEngine {
    pub fn new(llm: Arc<LlmClient>, store: Arc<DocumentStore>) -> Self {
        Self { llm, store }
    }

    /// Extract search terms from a question — handles hyphenated phrases and filters stop words.
    fn extract_keywords(question: &str) -> Vec<String> {
        let mut keywords = Vec::new();

        for word in question.split_whitespace() {
            // Strip punctuation
            let clean: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if clean.is_empty() {
                continue;
            }

            // Keep hyphenated terms as-is (e.g., "private-ip")
            if clean.contains('-') || clean.contains('_') {
                keywords.push(clean.to_lowercase());
                // Also add the parts individually
                for part in clean.split(|c: char| c == '-' || c == '_') {
                    if part.len() > 2 && !STOP_WORDS.contains(&part.to_lowercase().as_str()) {
                        keywords.push(part.to_lowercase());
                    }
                }
            } else if clean.len() > 2 && !STOP_WORDS.contains(&clean.to_lowercase().as_str()) {
                keywords.push(clean.to_lowercase());
            }
        }

        // Deduplicate while preserving order
        let mut seen = std::collections::HashSet::new();
        keywords.retain(|k| seen.insert(k.clone()));
        keywords.truncate(6);
        keywords
    }

    /// Build a lightweight bootstrap: one search + one file read.
    /// Heavy exploration is left to the LLM's own iterations.
    fn build_bootstrap_code(docs: &[DocMeta], question: &str) -> String {
        let doc_id = &docs[0].id;
        let keywords = Self::extract_keywords(question);

        // Use search_document (single scan, OR keyword matching, ranked by overlap)
        // instead of N separate grep calls
        let search_query = keywords.join(" ");

        format!(
            r#"doc_id = "{doc_id}"

# Search for relevant content (single pass, ranked by keyword overlap)
results = search_document(doc_id, "{search_query}", 5)
print(f"=== {{len(results)}} search results for: {search_query} ===")
for r in results:
    print(f"\n[offset={{r['offset']}}, matches={{r['match_count']}}]")
    print(r["content"])
print()

# Show relevant files by name
files = list_files(doc_id)
keywords = {keywords}
relevant = [(sum(1 for k in keywords if k in f["name"].lower()), f) for f in files]
relevant = [(s,f) for s,f in relevant if s > 0]
relevant.sort(key=lambda x: -x[0])
print(f"=== {{len(files)}} total files, {{len(relevant)}} match keywords by name ===")
for score, f in relevant[:10]:
    print(f"  [offset={{f['offset']}}] {{f['name']}}")

# Auto-read the best matching file
if relevant:
    best = relevant[0][1]["name"]
    print(f"\n=== Reading: {{best}} ===")
    content = read_file(doc_id, best)
    print(content[:3000])
    if len(content) > 3000:
        print(f"... [{{len(content)}} total chars]")
elif results:
    # No filename match — read around the best search result
    best_offset = max(results[0]["offset"] - 500, 0)
    print(f"\n=== Content around best match (offset {{best_offset}}) ===")
    print(get_section(doc_id, best_offset, 3000))
"#,
            doc_id = doc_id,
            search_query = search_query,
            keywords = format!("{:?}", keywords),
        )
    }

    pub async fn query(
        &self,
        topic: &str,
        question: &str,
        max_iterations: u32,
        min_code_executions: u32,
        min_answer_len: usize,
    ) -> Result<RlmResponse> {
        let topic_docs = self.store.list_by_label(topic).await?;
        if topic_docs.is_empty() {
            return Ok(RlmResponse {
                answer: format!(
                    "No documents found for topic '{}'. Use `/edgar ingest` to add some first.",
                    topic
                ),
                iterations: 0,
                sources: vec![],
                evidence: vec![],
            });
        }

        info!(topic, doc_count = topic_docs.len(), "Starting RLM query");

        let session =
            PersistentSession::spawn(self.store.clone(), self.llm.clone(), topic_docs.clone());

        let doc_summary: Vec<String> = topic_docs
            .iter()
            .map(|d| {
                format!(
                    "  - doc_id=\"{}\" name=\"{}\" source=\"{}\" size={}",
                    d.id, d.name, d.source, d.size
                )
            })
            .collect();
        let system_with_docs = format!(
            "{}\n\nDocuments loaded for topic '{}':\n{}",
            prompts::SYSTEM_PROMPT,
            topic,
            doc_summary.join("\n")
        );

        // Bootstrap: auto-execute an initial read so the LLM sees proof the REPL works
        let bootstrap_code = Self::build_bootstrap_code(&topic_docs, question);
        let bootstrap_output = session.execute(&bootstrap_code).await?;
        debug!(
            output_len = bootstrap_output.len(),
            "─── Bootstrap Output ───"
        );
        for line in bootstrap_output.lines().take(20) {
            debug!("  │ {}", line);
        }

        let bootstrap_output_msg = if bootstrap_output.len() > 4000 {
            format!(
                "{}...\n[truncated, {} total chars — use grep() or read_file() for more]",
                &bootstrap_output[..4000],
                bootstrap_output.len()
            )
        } else {
            bootstrap_output.clone()
        };

        let mut messages = vec![
            Message {
                role: "system".to_string(),
                content: system_with_docs,
            },
            // Show the bootstrap as an assistant code block + user output
            // so the LLM sees a working example in its own conversation
            Message {
                role: "assistant".to_string(),
                content: format!("I'll start by reading the documents.\n\n```repl\n{}\n```", bootstrap_code),
            },
            Message {
                role: "user".to_string(),
                content: format!("[REPL Output]\n{}", bootstrap_output_msg),
            },
            Message {
                role: "user".to_string(),
                content: format!(
                    "The REPL is working. Now answer this question using the document content above \
                    and further searches as needed: {}",
                    question
                ),
            },
        ];

        let sources: Vec<String> = topic_docs.iter().map(|d| d.source.clone()).collect();
        let mut code_executions = 1u32; // bootstrap counts as one
                                        // Accumulate substantive REPL outputs as evidence
        let mut evidence: Vec<String> = Vec::new();
        if bootstrap_output.len() > 50 && !bootstrap_output.starts_with("Error:") {
            evidence.push(bootstrap_output);
        }

        for i in 0..max_iterations {
            let iteration = i + 1;

            let response = self.llm.chat(&messages, None).await?;

            debug!(
                iteration,
                response_len = response.len(),
                "─── LLM Response ───"
            );
            for line in response.lines().take(50) {
                debug!("  │ {}", line);
            }
            if response.lines().count() > 50 {
                debug!("  │ ... ({} lines total)", response.lines().count());
            }

            let cmd = Command::parse(&response);
            debug!(iteration, cmd = ?cmd, "Parsed command");

            match cmd {
                Command::Final(answer) => {
                    // Gate 1: enough code executions?
                    if code_executions < min_code_executions {
                        debug!(
                            iteration,
                            code_executions, "FINAL rejected — not enough code runs"
                        );
                        messages.push(Message {
                            role: "assistant".to_string(),
                            content: response,
                        });
                        messages.push(Message {
                            role: "user".to_string(),
                            content: format!(
                                "You only ran {} code block(s). Read the actual document content first. \
                                Use get_section(documents[0][\"doc_id\"], 0, 5000) to read the start, \
                                then search for terms related to my question. Print everything you read.",
                                code_executions
                            ),
                        });
                        continue;
                    }

                    // Gate 2: answer long enough to be substantive?
                    if answer.len() < min_answer_len {
                        debug!(
                            iteration,
                            answer_len = answer.len(),
                            "FINAL rejected — too short"
                        );
                        messages.push(Message {
                            role: "assistant".to_string(),
                            content: response,
                        });
                        messages.push(Message {
                            role: "user".to_string(),
                            content: "Your answer is too brief. Include specific details from the \
                                document content you read — quote file names, function signatures, \
                                configuration fields, or other concrete information you found."
                                .to_string(),
                        });
                        continue;
                    }

                    info!(
                        iteration,
                        code_executions,
                        answer_len = answer.len(),
                        "RLM complete"
                    );
                    debug!("Final answer: {}", &answer[..answer.len().min(500)]);
                    return Ok(RlmResponse {
                        answer,
                        iterations: iteration,
                        sources,
                        evidence,
                    });
                }
                Command::RunCode(code) => {
                    debug!(iteration, "─── Executing Code ───");
                    for line in code.lines() {
                        debug!("  │ {}", line);
                    }

                    messages.push(Message {
                        role: "assistant".to_string(),
                        content: response,
                    });

                    let output = session.execute(&code).await?;
                    code_executions += 1;

                    debug!(
                        iteration,
                        code_executions,
                        output_len = output.len(),
                        "─── Code Output ───"
                    );
                    for line in output.lines().take(30) {
                        debug!("  │ {}", line);
                    }
                    if output.lines().count() > 30 {
                        debug!("  │ ... ({} lines total)", output.lines().count());
                    }

                    // Collect substantive outputs as evidence (skip empty/error-only)
                    if output.len() > 50 && !output.starts_with("Error:") {
                        evidence.push(output.clone());
                    }

                    let output_msg = if output.is_empty() {
                        "[No output — use print() to see results]".to_string()
                    } else if output.len() > 4000 {
                        format!(
                            "{}...\n[truncated, {} total chars — narrow your search or read smaller sections]",
                            &output[..4000],
                            output.len()
                        )
                    } else {
                        output
                    };

                    messages.push(Message {
                        role: "user".to_string(),
                        content: format!("[REPL Output]\n{}", output_msg),
                    });
                }
                Command::InvalidCommand => {
                    debug!(iteration, "InvalidCommand — nudging");
                    messages.push(Message {
                        role: "assistant".to_string(),
                        content: response,
                    });
                    messages.push(Message {
                        role: "user".to_string(),
                        content: format!(
                            "I need you to write Python code to read the documents. Wrap code in \
                            ```repl\\n...\\n```. There are {} document(s) in `documents`. \
                            Try: print(get_section(documents[0][\"doc_id\"], 0, 5000))",
                            topic_docs.len()
                        ),
                    });
                }
            }
        }

        // Max iterations — synthesize from evidence
        warn!(
            code_executions,
            evidence_count = evidence.len(),
            "RLM hit max iterations"
        );

        let answer = self
            .synthesize_from_evidence(&mut messages, &evidence, question)
            .await?;

        // Validate before returning
        let answer = self.validate_answer(answer, &evidence, question).await?;

        Ok(RlmResponse {
            answer,
            iterations: max_iterations,
            sources,
            evidence,
        })
    }

    /// Synthesize an answer from collected evidence when the loop exhausts iterations.
    async fn synthesize_from_evidence(
        &self,
        messages: &mut Vec<Message>,
        evidence: &[String],
        question: &str,
    ) -> Result<String> {
        if !evidence.is_empty() {
            let evidence_summary = evidence
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let truncated = if e.len() > 2000 {
                        &e[..2000]
                    } else {
                        e.as_str()
                    };
                    format!("--- Evidence {} ---\n{}", i + 1, truncated)
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            messages.push(Message {
                role: "user".to_string(),
                content: format!(
                    "Here is all the document content collected during this session:\n\n{}\n\n\
                    Based ONLY on this evidence, answer the question: {}\n\
                    Include specific details, names, and quotes from the text above. Wrap in FINAL(...).",
                    evidence_summary, question
                ),
            });
        } else {
            messages.push(Message {
                role: "user".to_string(),
                content: format!(
                    "Summarize everything you found about: {}\nWrap in FINAL(...).",
                    question
                ),
            });
        }

        let response = self.llm.chat(messages, None).await?;
        debug!("Synthesized: {}", &response[..response.len().min(500)]);

        Ok(match Command::parse(&response) {
            Command::Final(a) => a,
            _ => response,
        })
    }

    /// Validate an answer before returning it to the user. If it looks broken
    /// (refusal, empty, apology), attempt to rescue from evidence.
    async fn validate_answer(
        &self,
        answer: String,
        evidence: &[String],
        question: &str,
    ) -> Result<String> {
        // Check for known broken patterns
        let answer_lower = answer.to_lowercase();
        let is_broken = answer.trim().is_empty()
            || BROKEN_ANSWER_PATTERNS
                .iter()
                .any(|p| answer_lower.contains(p));

        if !is_broken {
            return Ok(answer);
        }

        warn!(
            answer_len = answer.len(),
            "Answer validation failed — attempting rescue"
        );

        // If we have evidence, build an answer directly from it
        if !evidence.is_empty() {
            let evidence_text: String = evidence
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, e)| {
                    let truncated = if e.len() > 3000 {
                        &e[..3000]
                    } else {
                        e.as_str()
                    };
                    format!("--- Source {} ---\n{}", i + 1, truncated)
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            let rescue_messages = vec![
                Message {
                    role: "system".to_string(),
                    content: "You are a helpful assistant. Answer the question using ONLY the \
                        provided document excerpts. Be specific and quote the text directly."
                        .to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: format!(
                        "Document excerpts:\n\n{}\n\nQuestion: {}\n\n\
                        Answer with specific details from the excerpts above.",
                        evidence_text, question
                    ),
                },
            ];

            let rescue = self.llm.chat(&rescue_messages, None).await?;
            info!(rescue_len = rescue.len(), "Rescue answer generated");

            // Strip FINAL() wrapper if present
            Ok(match Command::parse(&rescue) {
                Command::Final(a) => a,
                _ => rescue,
            })
        } else {
            // No evidence at all — return an honest failure message
            Ok(format!(
                "I wasn't able to find relevant information about \"{}\" in the ingested documents. \
                The documents may not contain content related to this question. \
                Try rephrasing or checking `/edgar sources` to see what's available.",
                question
            ))
        }
    }
}
