use std::sync::Arc;

use anyhow::Result;
use pyo3::prelude::*;
use pyo3::types::{PyCFunction, PyDict, PyList};
use tokio::runtime::Handle;
use tracing::{debug, warn};

use crate::docs::types::DocMeta;
use crate::docs::DocumentStore;
use crate::llm::LlmClient;

pub const BLOCKED: &[&str] = &[
    "__import__",
    "eval",
    "exec",
    "compile",
    "open",
    "input",
    "globals",
    "locals",
    "breakpoint",
    "exit",
    "quit",
];

pub const ALLOWED: &[&str] = &[
    "print",
    "len",
    "str",
    "int",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "range",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sorted",
    "min",
    "max",
    "sum",
    "abs",
    "round",
    "type",
    "isinstance",
    "hasattr",
    "getattr",
    "repr",
    "format",
    "True",
    "False",
    "None",
    "any",
    "all",
    "reversed",
    "chr",
    "ord",
];

/// Request to execute code on the persistent Python thread.
struct ExecRequest {
    code: String,
    reply: std::sync::mpsc::Sender<Result<String>>,
}

/// A persistent Python execution session that maintains globals across code blocks.
/// Runs on a dedicated OS thread. Uses std::sync channels to avoid nested block_on.
pub struct PersistentSession {
    tx: std::sync::mpsc::Sender<ExecRequest>,
}

impl PersistentSession {
    /// Spawn a new persistent session. Python globals survive across execute() calls.
    pub fn spawn(store: Arc<DocumentStore>, llm: Arc<LlmClient>, docs: Vec<DocMeta>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<ExecRequest>();

        std::thread::spawn(move || {
            // Build a runtime for async bridging inside PyO3 closures.
            // CRITICAL: we do NOT call block_on for the recv loop — we use
            // std::sync::mpsc::recv() which is a plain OS block, keeping the
            // thread outside of any tokio runtime context. This lets the PyO3
            // closures safely call rt_handle.block_on() without nesting.
            let bridge_rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create bridge runtime");
            let rt_handle = bridge_rt.handle().clone();

            Python::with_gil(|py| {
                let globals = PyDict::new(py);

                if let Err(e) = setup_restricted_builtins(py, &globals) {
                    warn!("Failed to setup builtins: {}", e);
                    return;
                }
                if let Err(e) = inject_doc_functions(py, &globals, store, llm, rt_handle, &docs) {
                    warn!("Failed to inject doc functions: {}", e);
                    return;
                }

                debug!("Persistent Python session initialized");
                // Plain OS-level blocking recv — NOT inside a tokio runtime context
                while let Ok(req) = rx.recv() {
                    let result = execute_in_globals(py, &globals, &req.code);
                    let _ = req.reply.send(result);
                }
                debug!("Persistent Python session shutting down");
            });
        });

        Self { tx }
    }

    /// Execute code in the persistent session. Variables from previous calls are available.
    pub async fn execute(&self, code: &str) -> Result<String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.tx
            .send(ExecRequest {
                code: code.to_string(),
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("Python session thread died"))?;

        // Await reply without blocking the tokio runtime
        tokio::task::spawn_blocking(move || {
            reply_rx
                .recv()
                .map_err(|_| anyhow::anyhow!("Python session reply channel closed"))?
        })
        .await?
    }
}

/// Execute code within existing globals, capturing stdout.
fn execute_in_globals(py: Python<'_>, globals: &Bound<'_, PyDict>, code: &str) -> Result<String> {
    let io_module = py.import("io")?;
    let string_io = io_module.getattr("StringIO")?.call0()?;
    let sys = py.import("sys")?;
    let old_stdout = sys.getattr("stdout")?;
    sys.setattr("stdout", &string_io)?;

    let code_cstr = std::ffi::CString::new(code.as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid code string: {}", e))?;
    let result = py.run(&code_cstr, Some(globals), None);

    sys.setattr("stdout", old_stdout)?;

    let output: String = string_io.call_method0("getvalue")?.extract()?;

    match result {
        Ok(_) => {
            debug!(output_len = output.len(), "Python executed successfully");
            Ok(output)
        }
        Err(e) => {
            warn!("Python execution error: {}", e);
            Ok(format!("{}\nError: {}", output, e))
        }
    }
}

/// Set up restricted builtins — whitelist approach.
fn setup_restricted_builtins(py: Python<'_>, globals: &Bound<'_, PyDict>) -> PyResult<()> {
    let builtins = py.import("builtins")?;
    let restricted = PyDict::new(py);

    for name in ALLOWED {
        if let Ok(obj) = builtins.getattr(*name) {
            restricted.set_item(*name, obj)?;
        }
    }

    for name in BLOCKED {
        restricted.set_item(*name, py.None())?;
    }

    globals.set_item("__builtins__", restricted)?;
    Ok(())
}

/// Inject document access functions and variables into Python globals.
fn inject_doc_functions(
    py: Python<'_>,
    globals: &Bound<'_, PyDict>,
    store: Arc<DocumentStore>,
    llm: Arc<LlmClient>,
    rt: Handle,
    docs: &[DocMeta],
) -> PyResult<()> {
    // Inject `documents` variable
    let doc_list = PyList::empty(py);
    for doc in docs {
        let d = PyDict::new(py);
        d.set_item("doc_id", &doc.id)?;
        d.set_item("name", &doc.name)?;
        d.set_item("source", &doc.source)?;
        d.set_item("size", doc.size)?;
        doc_list.append(d)?;
    }
    globals.set_item("documents", doc_list)?;

    // list_documents()
    let docs_clone: Vec<DocMeta> = docs.to_vec();
    let list_documents = PyCFunction::new_closure(
        py,
        Some(c"list_documents"),
        None,
        move |_args: &Bound<'_, pyo3::types::PyTuple>,
              _kwargs: Option<&Bound<'_, PyDict>>|
              -> PyResult<PyObject> {
            Python::with_gil(|py| {
                let result = PyList::empty(py);
                for doc in &docs_clone {
                    let d = PyDict::new(py);
                    d.set_item("doc_id", &doc.id)?;
                    d.set_item("name", &doc.name)?;
                    d.set_item("source", &doc.source)?;
                    d.set_item("size", doc.size)?;
                    result.append(d)?;
                }
                Ok(result.into_any().unbind())
            })
        },
    )?;
    globals.set_item("list_documents", list_documents)?;

    // get_section(doc_id, offset, length)
    let store_gs = store.clone();
    let rt_gs = rt.clone();
    let get_section = PyCFunction::new_closure(
        py,
        Some(c"get_section"),
        None,
        move |args: &Bound<'_, pyo3::types::PyTuple>,
              _kwargs: Option<&Bound<'_, PyDict>>|
              -> PyResult<String> {
            let doc_id: String = args.get_item(0)?.extract()?;
            let offset: usize = args.get_item(1)?.extract()?;
            let length: usize = args.get_item(2)?.extract()?;
            tracing::debug!(doc_id = %doc_id, offset, length, "PyO3: get_section");
            rt_gs
                .block_on(store_gs.get_section(&doc_id, offset, length))
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
        },
    )?;
    globals.set_item("get_section", get_section)?;

    // search_document(doc_id, query, max_results=5)
    let store_sd = store.clone();
    let rt_sd = rt.clone();
    let search_document = PyCFunction::new_closure(
        py,
        Some(c"search_document"),
        None,
        move |args: &Bound<'_, pyo3::types::PyTuple>,
              _kwargs: Option<&Bound<'_, PyDict>>|
              -> PyResult<PyObject> {
            let doc_id: String = args.get_item(0)?.extract()?;
            let query: String = args.get_item(1)?.extract()?;
            let max_results: usize = if args.len() > 2 {
                args.get_item(2)?.extract().unwrap_or(5)
            } else {
                5
            };
            tracing::debug!(doc_id = %doc_id, query = %query, max_results, "PyO3: search_document");
            let excerpts = rt_sd
                .block_on(store_sd.search(&doc_id, &query, max_results))
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
            tracing::debug!(results = excerpts.len(), "PyO3: search_document result");
            Python::with_gil(|py| {
                let result = PyList::empty(py);
                for ex in &excerpts {
                    let d = PyDict::new(py);
                    d.set_item("doc_id", &ex.doc_id)?;
                    d.set_item("offset", ex.offset)?;
                    d.set_item("content", &ex.content)?;
                    d.set_item("match_count", ex.match_count)?;
                    result.append(d)?;
                }
                Ok(result.into_any().unbind())
            })
        },
    )?;
    globals.set_item("search_document", search_document)?;

    // grep(doc_id, pattern, context=3, max_results=10) — search with context lines
    let store_gr = store.clone();
    let rt_gr = rt.clone();
    let grep = PyCFunction::new_closure(
        py,
        Some(c"grep"),
        None,
        move |args: &Bound<'_, pyo3::types::PyTuple>,
              _kwargs: Option<&Bound<'_, PyDict>>|
              -> PyResult<PyObject> {
            let doc_id: String = args.get_item(0)?.extract()?;
            let pattern: String = args.get_item(1)?.extract()?;
            let context_lines: usize = if args.len() > 2 {
                args.get_item(2)?.extract().unwrap_or(3)
            } else {
                3
            };
            let max_results: usize = if args.len() > 3 {
                args.get_item(3)?.extract().unwrap_or(10)
            } else {
                10
            };
            tracing::debug!(doc_id = %doc_id, pattern = %pattern, context_lines, "PyO3: grep");

            let content = rt_gr
                .block_on(store_gr.get_content(&doc_id))
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
            let text = String::from_utf8_lossy(&content);
            let lines: Vec<&str> = text.lines().collect();

            // Match lines containing the pattern (case-insensitive)
            let pattern_lower = pattern.to_lowercase();
            let mut matches: Vec<(usize, String)> = Vec::new();
            let mut last_end: usize = 0; // track to avoid overlapping contexts

            for (idx, line) in lines.iter().enumerate() {
                if line.to_lowercase().contains(&pattern_lower) {
                    let start = idx.saturating_sub(context_lines).max(last_end);
                    let end = (idx + context_lines + 1).min(lines.len());

                    let context_block: String = lines[start..end]
                        .iter()
                        .enumerate()
                        .map(|(i, l)| {
                            let ln = start + i + 1;
                            if start + i == idx {
                                format!(">> L{}: {}", ln, l)
                            } else {
                                format!("   L{}: {}", ln, l)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    matches.push((idx + 1, context_block));
                    last_end = end;

                    if matches.len() >= max_results {
                        break;
                    }
                }
            }

            tracing::debug!(results = matches.len(), "PyO3: grep result");
            Python::with_gil(|py| {
                let result = PyList::empty(py);
                for (line_num, context) in &matches {
                    let d = PyDict::new(py);
                    d.set_item("line", *line_num)?;
                    d.set_item("context", context)?;
                    result.append(d)?;
                }
                Ok(result.into_any().unbind())
            })
        },
    )?;
    globals.set_item("grep", grep)?;

    // list_files(doc_id) — extract file/section headers from ingested document
    let store_lf = store.clone();
    let rt_lf = rt.clone();
    let list_files = PyCFunction::new_closure(
        py,
        Some(c"list_files"),
        None,
        move |args: &Bound<'_, pyo3::types::PyTuple>,
              _kwargs: Option<&Bound<'_, PyDict>>|
              -> PyResult<PyObject> {
            let doc_id: String = args.get_item(0)?.extract()?;
            tracing::debug!(doc_id = %doc_id, "PyO3: list_files");

            let files = rt_lf
                .block_on(store_lf.list_files(&doc_id))
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

            tracing::debug!(file_count = files.len(), "PyO3: list_files result");
            Python::with_gil(|py| {
                let result = PyList::empty(py);
                for (offset, name) in &files {
                    let d = PyDict::new(py);
                    d.set_item("offset", *offset)?;
                    d.set_item("name", name)?;
                    result.append(d)?;
                }
                Ok(result.into_any().unbind())
            })
        },
    )?;
    globals.set_item("list_files", list_files)?;

    // read_file(doc_id, filename) — read a specific file/section by name
    let store_rf = store.clone();
    let rt_rf = rt.clone();
    let read_file = PyCFunction::new_closure(
        py,
        Some(c"read_file"),
        None,
        move |args: &Bound<'_, pyo3::types::PyTuple>,
              _kwargs: Option<&Bound<'_, PyDict>>|
              -> PyResult<String> {
            let doc_id: String = args.get_item(0)?.extract()?;
            let filename: String = args.get_item(1)?.extract()?;
            tracing::debug!(doc_id = %doc_id, filename = %filename, "PyO3: read_file");

            let files = rt_rf
                .block_on(store_rf.list_files(&doc_id))
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

            // Find this file's offset and the next file's offset
            let filename_lower = filename.to_lowercase();
            let mut target_offset = None;
            let mut next_offset = None;

            for (i, (offset, name)) in files.iter().enumerate() {
                if name.to_lowercase().contains(&filename_lower) {
                    target_offset = Some(*offset);
                    if i + 1 < files.len() {
                        next_offset = Some(files[i + 1].0);
                    }
                    break;
                }
            }

            let Some(start) = target_offset else {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "File '{}' not found. Use list_files() to see available files.",
                    filename
                )));
            };

            // Read until next file header or max 20K chars
            let max_len = 20_000;
            let length = if let Some(end) = next_offset {
                (end - start).min(max_len)
            } else {
                max_len
            };

            rt_rf
                .block_on(store_rf.get_section(&doc_id, start, length))
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
        },
    )?;
    globals.set_item("read_file", read_file)?;

    // llm_query(prompt)
    let llm_q = llm.clone();
    let rt_lq = rt.clone();
    let llm_query = PyCFunction::new_closure(
        py,
        Some(c"llm_query"),
        None,
        move |args: &Bound<'_, pyo3::types::PyTuple>,
              _kwargs: Option<&Bound<'_, PyDict>>|
              -> PyResult<String> {
            let prompt: String = args.get_item(0)?.extract()?;
            tracing::debug!(prompt_len = prompt.len(), "PyO3: llm_query");
            rt_lq
                .block_on(llm_q.sub_query(&prompt))
                .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
        },
    )?;
    globals.set_item("llm_query", llm_query)?;

    Ok(())
}
