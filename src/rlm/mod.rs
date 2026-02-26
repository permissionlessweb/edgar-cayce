pub mod citations;
pub mod exec;
pub mod prompts;
pub mod repl;

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, info, warn};

use crate::docs::types::{DocMeta, QaRecord};
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
    /// Public URLs extracted from markdown links in the answer
    pub cited_urls: Vec<String>,
}

/// Extract URLs from markdown links `[text](url)` in the answer text.
fn extract_cited_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        if let Some(end) = after.find(')') {
            let url = after[..end].trim();
            if url.starts_with("http") && !urls.contains(&url.to_string()) {
                urls.push(url.to_string());
            }
        }
        rest = &rest[start + 2..];
    }
    urls
}

/// Which exploration strategy a loop should use.
#[derive(Debug, Clone, Copy)]
enum ExplorationStrategy {
    Broad,
    Deep,
}

/// Internal result from a single exploration loop — not exposed publicly.
struct LoopResult {
    answer: String,
    iterations: u32,
    evidence: Vec<String>,
    cited_urls: Vec<String>,
    /// true = natural FINAL(), false = max-iteration synthesis
    was_final: bool,
    /// The sub-question this loop investigated (None for atomic/single-loop queries).
    sub_question: Option<String>,
}

/// Parse decomposition LLM response into sub-questions.
/// Returns empty vec for ATOMIC questions.
fn parse_decomposition(response: &str) -> Vec<String> {
    if response.contains("ATOMIC") {
        return Vec::new();
    }

    response
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("SUB(") {
                // Find matching close paren using depth counting (like FINAL parser)
                let after = &trimmed[4..];
                let mut depth = 1i32;
                let mut end = None;
                for (i, ch) in after.char_indices() {
                    match ch {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(i);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let content = match end {
                    Some(e) => &after[..e],
                    None => after.trim_end_matches(')'),
                };
                let q = content.trim();
                if q.is_empty() {
                    None
                } else {
                    Some(q.to_string())
                }
            } else {
                None
            }
        })
        .collect()
}

/// Combine evidence and URLs from multiple loop results, deduplicating.
fn combine_loop_artifacts(results: &[LoopResult]) -> (Vec<String>, Vec<String>) {
    let mut combined_evidence: Vec<String> = Vec::new();
    let mut seen_prefixes = HashSet::new();
    let mut combined_urls: Vec<String> = Vec::new();
    let mut seen_urls = HashSet::new();

    for r in results {
        for ev in &r.evidence {
            let prefix: String = ev.chars().take(200).collect();
            if seen_prefixes.insert(prefix) {
                combined_evidence.push(ev.clone());
            }
        }
        for url in &r.cited_urls {
            if seen_urls.insert(url.clone()) {
                combined_urls.push(url.clone());
            }
        }
    }

    (combined_evidence, combined_urls)
}

#[derive(Clone)]
pub struct RlmEngine {
    llm: Arc<LlmClient>,
    store: Arc<DocumentStore>,
}

impl RlmEngine {
    pub fn new(llm: Arc<LlmClient>, store: Arc<DocumentStore>) -> Self {
        Self { llm, store }
    }

    /// Fire-and-forget Q/A storage. Logs errors but never fails the response.
    async fn store_qa_record(
        &self,
        topic: &str,
        question: &str,
        response: &RlmResponse,
        doc_ids: Vec<String>,
    ) {
        let id = blake3::hash(format!("{}{}", topic, question).as_bytes())
            .to_hex()
            .to_string();
        let record = QaRecord {
            id,
            topic: topic.to_string(),
            question: question.to_string(),
            answer: response.answer.clone(),
            cited_urls: response.cited_urls.clone(),
            doc_ids,
            evidence: response.evidence.clone(),
            iterations: response.iterations,
            timestamp: chrono::Utc::now().timestamp(),
        };
        if let Err(e) = self.store.store_qa(&record).await {
            warn!(error = %e, "Failed to store Q/A record");
        }
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
        let mut seen = HashSet::new();
        keywords.retain(|k| seen.insert(k.clone()));
        keywords.truncate(6);
        keywords
    }

    /// Build a broad bootstrap: search_document + file scan + read best match (3000 chars).
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

    /// Build a deep bootstrap: grep with high context + read best match at 6000 chars.
    fn build_deep_bootstrap_code(docs: &[DocMeta], question: &str) -> String {
        let doc_id = &docs[0].id;
        let keywords = Self::extract_keywords(question);
        let grep_pattern = keywords.join("|");

        format!(
            r#"doc_id = "{doc_id}"

# Deep grep with high context on top keywords
hits = grep(doc_id, r"(?i){grep_pattern}", 8, 30)
print(f"=== {{len(hits)}} grep hits for: {grep_pattern} ===")
for h in hits[:10]:
    print(f"\n[line {{h['line']}}]")
    print(h["context"])

# List files to find best match
files = list_files(doc_id)
keywords = {keywords}
relevant = [(sum(1 for k in keywords if k in f["name"].lower()), f) for f in files]
relevant = [(s,f) for s,f in relevant if s > 0]
relevant.sort(key=lambda x: -x[0])
print(f"\n=== {{len(files)}} total files, {{len(relevant)}} match keywords ===")
for score, f in relevant[:5]:
    print(f"  [offset={{f['offset']}}] {{f['name']}}")

# Deep-read the best matching file (6000 chars)
if relevant:
    best = relevant[0][1]["name"]
    print(f"\n=== Deep reading: {{best}} ===")
    content = read_file(doc_id, best)
    print(content[:6000])
    if len(content) > 6000:
        print(f"... [{{len(content)}} total chars]")
elif hits:
    # Read around the best grep hit
    best_line = hits[0]["line"]
    approx_offset = max(best_line * 80 - 1000, 0)
    print(f"\n=== Content around best hit (approx offset {{approx_offset}}) ===")
    print(get_section(doc_id, approx_offset, 6000))
"#,
            doc_id = doc_id,
            grep_pattern = grep_pattern,
            keywords = format!("{:?}", keywords),
        )
    }

    // ─── Phase 1: Decomposition ──────────────────────────────────────────

    /// Analyze a question and decide whether to decompose it into parallel sub-investigations.
    /// Returns empty Vec for atomic questions, or a list of focused sub-questions.
    async fn decompose_question(
        &self,
        question: &str,
        topic_docs: &[DocMeta],
        max_subs: u32,
    ) -> Result<Vec<String>> {
        let doc_names: Vec<String> = topic_docs
            .iter()
            .map(|d| format!("\"{}\" ({})", d.name, d.source))
            .collect();

        let messages = vec![
            Message {
                role: "system".to_string(),
                content: prompts::DECOMPOSE_PROMPT.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: format!(
                    "Available documents: {}\nMaximum sub-questions: {}\n\nQuestion: {}",
                    doc_names.join(", "),
                    max_subs,
                    question,
                ),
            },
        ];

        let response = self.llm.chat(&messages, None).await?;
        debug!(response = %response, "Decomposition response");

        let subs = parse_decomposition(&response);

        if subs.is_empty() {
            info!("Question classified as ATOMIC");
        } else {
            info!(sub_count = subs.len(), "Question decomposed");
            for (i, sq) in subs.iter().enumerate() {
                info!(index = i, sub_question = %sq, "Sub-question");
            }
        }

        Ok(subs)
    }

    // ─── Phase 2: Exploration loop (single or sub) ───────────────────────

    /// Run a single exploration loop with its own Python session and strategy.
    ///
    /// When `original_question` is provided, this loop is a focused sub-investigation:
    /// the system prompt includes sub-loop context and the `question` param is the
    /// sub-question to investigate.
    async fn run_exploration_loop(
        &self,
        topic_docs: &[DocMeta],
        topic: &str,
        question: &str,
        max_iterations: u32,
        min_code_executions: u32,
        min_answer_len: usize,
        strategy: ExplorationStrategy,
        original_question: Option<&str>,
    ) -> Result<LoopResult> {
        let session =
            PersistentSession::spawn(self.store.clone(), self.llm.clone(), topic_docs.to_vec());

        let doc_summary: Vec<String> = topic_docs
            .iter()
            .map(|d| {
                let mut line = format!(
                    "  - doc_id=\"{}\" name=\"{}\" source=\"{}\" size={}",
                    d.id, d.name, d.source, d.size
                );
                if let Some(ctx) = &d.url_context {
                    line.push_str(&format!("\n    URL_CONTEXT: {}", ctx));
                }
                line
            })
            .collect();

        let strategy_appendix = match strategy {
            ExplorationStrategy::Broad => prompts::BROAD_APPENDIX,
            ExplorationStrategy::Deep => prompts::DEEP_APPENDIX,
        };

        // Build system prompt — add sub-loop context when running a focused sub-investigation
        let sub_loop_context = if let Some(oq) = original_question {
            format!(
                "{}\n\nOriginal question (for context): {}\nYour focused sub-question: {}",
                prompts::SUB_LOOP_APPENDIX,
                oq,
                question,
            )
        } else {
            String::new()
        };

        let system_with_docs = format!(
            "{}\n\nDocuments loaded for topic '{}':\n{}\n{}{}",
            prompts::SYSTEM_PROMPT,
            topic,
            doc_summary.join("\n"),
            strategy_appendix,
            sub_loop_context,
        );

        // Strategy-specific bootstrap code (uses question keywords for search)
        let bootstrap_code = match strategy {
            ExplorationStrategy::Broad => Self::build_bootstrap_code(topic_docs, question),
            ExplorationStrategy::Deep => Self::build_deep_bootstrap_code(topic_docs, question),
        };

        let bootstrap_output = session.execute(&bootstrap_code).await?;
        debug!(
            ?strategy,
            is_sub = original_question.is_some(),
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
            Message {
                role: "assistant".to_string(),
                content: format!(
                    "I'll start by reading the documents.\n\n```repl\n{}\n```",
                    bootstrap_code
                ),
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

        let mut code_executions = 1u32; // bootstrap counts as one
        let mut evidence: Vec<String> = Vec::new();
        if bootstrap_output.len() > 50 && !bootstrap_output.starts_with("Error:") {
            evidence.push(bootstrap_output);
        }

        for i in 0..max_iterations {
            let iteration = i + 1;
            let response = self.llm.chat(&messages, None).await?;

            debug!(
                ?strategy,
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
            debug!(?strategy, iteration, cmd = ?cmd, "Parsed command");

            match cmd {
                Command::Final(answer) => {
                    // Gate 1: enough code executions?
                    if code_executions < min_code_executions {
                        debug!(
                            ?strategy,
                            iteration, code_executions, "FINAL rejected — not enough code runs"
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
                            ?strategy,
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
                        ?strategy,
                        iteration,
                        code_executions,
                        answer_len = answer.len(),
                        is_sub = original_question.is_some(),
                        "Loop complete"
                    );
                    let mut cited_urls = extract_cited_urls(&answer);

                    // Enforce citations: resolve URLs from files the LLM actually read
                    let accessed = session.accessed_files();
                    let extra = citations::resolve_citations(&accessed, topic_docs, &cited_urls);
                    if !extra.is_empty() {
                        debug!(extra_count = extra.len(), "Programmatic citations added");
                        cited_urls.extend(extra);
                    }

                    return Ok(LoopResult {
                        answer,
                        iterations: iteration,
                        evidence,
                        cited_urls,
                        was_final: true,
                        sub_question: original_question.map(|_| question.to_string()),
                    });
                }
                Command::RunCode(code) => {
                    debug!(?strategy, iteration, "─── Executing Code ───");
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
                        ?strategy,
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
                    debug!(?strategy, iteration, "InvalidCommand — nudging");
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
            ?strategy,
            code_executions,
            evidence_count = evidence.len(),
            is_sub = original_question.is_some(),
            "Loop hit max iterations"
        );

        let answer = self
            .synthesize_from_evidence(&mut messages, &evidence, question)
            .await?;
        let answer = self.validate_answer(answer, &evidence, question).await?;
        let mut cited_urls = extract_cited_urls(&answer);

        // Enforce citations: resolve URLs from files the LLM actually read
        let accessed = session.accessed_files();
        let extra = citations::resolve_citations(&accessed, topic_docs, &cited_urls);
        if !extra.is_empty() {
            debug!(extra_count = extra.len(), "Programmatic citations added (synthesis)");
            cited_urls.extend(extra);
        }

        Ok(LoopResult {
            answer,
            iterations: max_iterations,
            evidence,
            cited_urls,
            was_final: false,
            sub_question: original_question.map(|_| question.to_string()),
        })
    }

    // ─── Phase 3: Synthesis ──────────────────────────────────────────────

    /// Synthesize a final answer from parallel sub-investigation results.
    /// Combines all sub-loop findings into a single LLM call that produces
    /// a unified answer addressing the original question.
    async fn synthesize_findings(
        &self,
        question: &str,
        results: &[LoopResult],
        sources: Vec<String>,
    ) -> Result<RlmResponse> {
        // Build the findings document from all sub-loop results
        let mut findings = String::new();
        for (i, r) in results.iter().enumerate() {
            let label = r
                .sub_question
                .as_deref()
                .unwrap_or("(general investigation)");
            findings.push_str(&format!("### Sub-Investigation {} — {}\n", i + 1, label));
            findings.push_str(&format!("**Findings:**\n{}\n\n", r.answer));

            if !r.evidence.is_empty() {
                findings.push_str("**Key Evidence:**\n");
                for (j, ev) in r.evidence.iter().take(3).enumerate() {
                    let truncated = if ev.len() > 1500 {
                        &ev[..1500]
                    } else {
                        ev.as_str()
                    };
                    findings.push_str(&format!("Evidence {}: {}\n\n", j + 1, truncated));
                }
            }
            findings.push_str("---\n\n");
        }

        let messages = vec![
            Message {
                role: "system".to_string(),
                content: prompts::SYNTHESIS_PROMPT.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: format!(
                    "**Original Question:** {}\n\n\
                     **Sub-Investigations Completed:**\n\n{}\n\n\
                     Synthesize a comprehensive answer. Wrap in FINAL(...).",
                    question, findings,
                ),
            },
        ];

        info!(
            sub_count = results.len(),
            findings_len = findings.len(),
            "Synthesizing from sub-investigations"
        );

        let response = self.llm.chat(&messages, None).await?;
        let answer = match Command::parse(&response) {
            Command::Final(a) => a,
            _ => response,
        };

        // Combine evidence and URLs from all sub-loops
        let (mut combined_evidence, mut combined_urls) = combine_loop_artifacts(results);

        // Also capture URLs from the synthesis answer itself
        let mut seen_urls: HashSet<String> = combined_urls.iter().cloned().collect();
        for url in extract_cited_urls(&answer) {
            if seen_urls.insert(url.clone()) {
                combined_urls.push(url);
            }
        }

        let answer = self
            .validate_answer(answer, &combined_evidence, question)
            .await?;

        // Also capture URLs from post-validation answer
        for url in extract_cited_urls(&answer) {
            if seen_urls.insert(url.clone()) {
                combined_urls.push(url);
            }
        }

        let iterations = results.iter().map(|r| r.iterations).max().unwrap_or(0);

        Ok(RlmResponse {
            answer,
            iterations,
            sources,
            evidence: combined_evidence,
            cited_urls: combined_urls,
        })
    }

    // ─── Orchestrator ────────────────────────────────────────────────────

    /// Orchestrate the full RLM pipeline: decompose → parallel sub-loops → synthesize.
    ///
    /// For atomic questions (no decomposition), runs a single exploration loop directly.
    /// For decomposable questions, spawns focused sub-loops in parallel, then synthesizes
    /// their findings into a unified answer.
    pub async fn query(
        &self,
        topic: &str,
        question: &str,
        max_iterations: u32,
        min_code_executions: u32,
        min_answer_len: usize,
        parallel_loops: u32,
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
                cited_urls: vec![],
            });
        }

        let sources: Vec<String> = topic_docs.iter().map(|d| d.source.clone()).collect();
        let doc_ids: Vec<String> = topic_docs.iter().map(|d| d.id.clone()).collect();

        let max_subs = parallel_loops.max(1);

        // ── Phase 1: Decompose ──
        let sub_questions = self
            .decompose_question(question, &topic_docs, max_subs)
            .await
            .unwrap_or_else(|e| {
                warn!("Decomposition failed, falling back to atomic: {e}");
                Vec::new()
            });

        if sub_questions.is_empty() {
            // ── Atomic: single exploration loop ──
            info!(
                topic,
                doc_count = topic_docs.len(),
                "Atomic question — single loop"
            );

            let result = self
                .run_exploration_loop(
                    &topic_docs,
                    topic,
                    question,
                    max_iterations,
                    min_code_executions,
                    min_answer_len,
                    ExplorationStrategy::Broad,
                    None,
                )
                .await?;

            let response = RlmResponse {
                answer: result.answer,
                iterations: result.iterations,
                sources,
                evidence: result.evidence,
                cited_urls: result.cited_urls,
            };
            self.store_qa_record(topic, question, &response, doc_ids)
                .await;
            return Ok(response);
        }

        // ── Phase 2: Parallel sub-loops ──
        let sub_count = sub_questions.len() as u32;
        let per_loop_iters = (max_iterations + sub_count - 1) / sub_count;
        // Sub-loops can produce shorter answers — the synthesis step produces the full answer
        let sub_min_answer = (min_answer_len / 2).max(50);
        let strategies = [ExplorationStrategy::Broad, ExplorationStrategy::Deep];

        info!(
            topic,
            doc_count = topic_docs.len(),
            sub_count,
            per_loop_iters,
            "Starting parallel sub-investigations"
        );

        let mut tasks = tokio::task::JoinSet::new();
        for (i, sub_q) in sub_questions.iter().enumerate() {
            let strategy = strategies[i % strategies.len()];
            let engine = self.clone();
            let docs = topic_docs.clone();
            let sq = sub_q.clone();
            let oq = question.to_string();
            let t = topic.to_string();
            tasks.spawn(async move {
                engine
                    .run_exploration_loop(
                        &docs,
                        &t,
                        &sq,
                        per_loop_iters,
                        min_code_executions,
                        sub_min_answer,
                        strategy,
                        Some(&oq),
                    )
                    .await
            });
        }

        // Collect sub-results
        let mut results: Vec<LoopResult> = Vec::new();
        let mut last_err = None;
        while let Some(join_result) = tasks.join_next().await {
            match join_result {
                Ok(Ok(r)) => {
                    info!(
                        sub_question = r.sub_question.as_deref().unwrap_or("?"),
                        answer_len = r.answer.len(),
                        iterations = r.iterations,
                        was_final = r.was_final,
                        "Sub-loop complete"
                    );
                    results.push(r);
                }
                Ok(Err(e)) => {
                    warn!("Sub-loop failed: {e}");
                    last_err = Some(e);
                }
                Err(e) => {
                    warn!("Sub-loop panicked: {e}");
                }
            }
        }

        if results.is_empty() {
            return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all sub-loops failed")));
        }

        info!(
            successful = results.len(),
            total = sub_questions.len(),
            "Sub-loops complete, synthesizing"
        );

        // ── Phase 3: Synthesize ──
        let response = self
            .synthesize_findings(question, &results, sources)
            .await?;

        self.store_qa_record(topic, question, &response, doc_ids)
            .await;
        Ok(response)
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
                "There was a catostrophic failure in my attempt to answer you question about \"{}\" \
                This has been saved to used to help avoid or possibly reinforce this error from occuring again.",
                question
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_decomposition_atomic() {
        assert!(parse_decomposition("ATOMIC").is_empty());
        assert!(parse_decomposition("This is ATOMIC question.").is_empty());
    }

    #[test]
    fn test_parse_decomposition_subs() {
        let input = "SUB(How does staking work?)\nSUB(What are slashing penalties?)";
        let subs = parse_decomposition(input);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0], "How does staking work?");
        assert_eq!(subs[1], "What are slashing penalties?");
    }

    #[test]
    fn test_parse_decomposition_nested_parens() {
        let input = "SUB(What is func(a, b) used for?)";
        let subs = parse_decomposition(input);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0], "What is func(a, b) used for?");
    }

    #[test]
    fn test_parse_decomposition_with_noise() {
        let input = "I'll decompose this:\nSUB(First question)\nSome noise\nSUB(Second question)\n";
        let subs = parse_decomposition(input);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0], "First question");
        assert_eq!(subs[1], "Second question");
    }

    #[test]
    fn test_parse_decomposition_empty_sub() {
        let input = "SUB()";
        let subs = parse_decomposition(input);
        assert!(subs.is_empty());
    }
}
