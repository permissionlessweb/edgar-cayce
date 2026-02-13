# Edgar — Discord Knowledge Bot

RLM-powered knowledge assistant for Discord. Ingest documents (GitHub repos, URLs), then ask questions answered through Python reasoning loops with document access.

## Inspiration

The following resources are

- rlm paper: <https://arxiv.org/pdf/2512.24601>
- rlm blog:  <https://www.primeintellect.ai/blog/rlm>
- rlm-rs: <https://github.com/zircote/rlm-rs>
- rig-rs: <https://github.com/joshua-mo-143/rig-rlm>
- <https://github.com/brainqub3/claude_code_RLM>
- @shanev for yapping about rlm on X
- The Akash Clubhouse lead for the patience and coordination

> Shout out to the people who coordinated these resources!

## Prerequisites

- Rust (nightly)
- Python 3.8+ (PyO3 links at build time)
- [just](https://github.com/casey/just) task runner
- A Discord bot token with slash command permissions
- An OpenAI-compatible LLM endpoint (LM Studio, Ollama, vLLM, etc.)

## Discord Bot Setup

1. Go to <https://discord.com/developers/applications>
2. Create a new application
3. Go to **Bot** tab, click **Reset Token**, copy the token
4. Under **Privileged Gateway Intents**, enable **Message Content Intent**
5. Go to **OAuth2 > URL Generator**:
   - Scopes: `bot`, `applications.commands`
   - Bot Permissions: `Send Messages`, `Create Public Threads`, `Use Slash Commands`
6. Open the generated URL to invite the bot to your server
7. Copy your server's Guild ID (right-click server name > Copy Server ID — requires Developer Mode in Discord settings)

## Configuration

Create a `.env` file in the project root:

```
DISCORD_TOKEN=your-bot-token-here
DISCORD_GUILD_ID=your-guild-id-here

# LLM endpoint (any OpenAI-compatible API)
LLM_BASE_URL=http://localhost:1234/v1
LLM_MODEL=qwen/qwen3-8b
LLM_SUB_MODEL=qwen/qwen3-8b
LLM_API_KEY=
```

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DISCORD_TOKEN` | yes | — | Bot token from Discord developer portal |
| `DISCORD_GUILD_ID` | no | — | Guild ID for instant slash command registration. Without it, global registration takes up to 1 hour |
| `LLM_BASE_URL` | no | `http://localhost:1234/v1` | OpenAI-compatible chat completions endpoint |
| `LLM_MODEL` | no | `qwen/qwen3-8b` | Primary model for reasoning loop |
| `LLM_SUB_MODEL` | no | same as `LLM_MODEL` | Model for `llm_query()` sub-calls from Python |
| `LLM_API_KEY` | no | — | API key. Leave empty for keyless/local endpoints |

## Build & Run

```bash
# Check prerequisites
just preflight

# Build
just build

# Run
just run
```

The bot will log `Bot connected as: Edgar#XXXX` when ready.

## Slash Commands

All commands are under the `/edgar` parent:

### `/edgar ingest`

Ingest a document into the knowledge store.

```
/edgar ingest url:https://github.com/akash-network/website label:akash-docs doc_type:documentation
/edgar ingest url:https://docs.example.com/api label:api-docs
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `url` | yes | GitHub repo URL or any web page |
| `label` | yes | Topic label (used to scope `/edgar ask` queries) |
| `doc_type` | no | `documentation` (default), `code`, or `minimal` — controls file filtering for GitHub repos |

### `/edgar ask`

Ask a question about ingested documents. The bot runs a multi-step reasoning loop: it searches documents, reads sections, and optionally calls a sub-LLM before answering.

```
/edgar ask topic:akash-docs question:What are the hardware requirements for an Akash provider?
```

| Parameter | Required | Description |
|-----------|----------|-------------|
| `topic` | yes | Label matching ingested documents (autocompletes) |
| `question` | yes | Your question |

The response includes iteration count and source list.

### `/edgar sources`

List all ingested documents grouped by topic.

```
/edgar sources
/edgar sources limit:50
```

### `/edgar clear`

Acknowledge session clear (stateless in this PoC).

### `/edgar thread`

Create a new Discord thread for conversation.

```
/edgar thread name:Akash Research
```

## LLM Endpoint Setup

Edgar works with any OpenAI-compatible `/v1/chat/completions` endpoint.

**LM Studio** (local):

```bash
# Start LM Studio, load a model, enable server on port 1234
LLM_BASE_URL=http://localhost:1234/v1
```

**Ollama**:

```bash
ollama serve
LLM_BASE_URL=http://localhost:11434
LLM_MODEL=qwen2.5:7b
```

**OpenAI**:

```bash
LLM_BASE_URL=https://api.openai.com/v1
LLM_MODEL=gpt-4o
LLM_API_KEY=sk-...
```

**vLLM**:

```bash
LLM_BASE_URL=http://localhost:8000/v1
LLM_MODEL=Qwen/Qwen2.5-7B-Instruct
```

## Data Storage

Documents are stored in `./data/docs/` using cnidarium (Merkle-tree backed KV store). Content is deduplicated by blake3 hash.

```bash
# Wipe all ingested documents
just clean-data
```

## Just Commands

```
just build          # Debug build
just build-release  # Release build
just run            # Run bot (debug)
just run-release    # Run bot (release)
just run-debug      # Run with RUST_LOG=debug
just watch          # Rebuild on file changes
just install        # Install as 'edgar' to ~/.cargo/bin
just preflight      # Validate env, python, LLM endpoint
just env            # Show current config
just test           # Run tests
just test-repl      # Run REPL parser tests
just clean          # Clean build artifacts
just clean-data     # Wipe document storage
just clean-all      # Clean build + data
just clippy         # Lint
just fmt            # Format
just ci             # Full CI pipeline
```

## Project Structure

```
src/
├── main.rs           # Startup, env vars, poise framework
├── state.rs          # AppState shared across commands
├── llm.rs            # OpenAI-compatible HTTP client
├── commands/
│   ├── mod.rs        # /edgar parent command
│   ├── ask.rs        # /edgar ask — RLM reasoning loop
│   ├── ingest.rs     # /edgar ingest — GitHub + URL
│   ├── sources.rs    # /edgar sources — list documents
│   └── manage.rs     # /edgar clear, /edgar thread
├── docs/
│   ├── mod.rs        # DocumentStore (cnidarium-backed)
│   ├── types.rs      # DocId, DocMeta, DocExcerpt
│   └── ingest.rs     # GitHub ingestion via githem-core
└── rlm/
    ├── mod.rs        # RlmEngine reasoning loop
    ├── repl.rs       # Command parser (code blocks, FINAL)
    ├── exec.rs       # PyO3 executor with sandboxed builtins
    └── prompts.rs    # System prompt for document-aware RLM
```

## How the RLM Works

1. User asks a question scoped to a topic
2. Engine loads documents matching that topic label
3. System prompt instructs the LLM to use Python code for document analysis
4. LLM outputs `\`\`\`repl ... \`\`\`` blocks which are executed in a PyO3 sandbox
5. Sandbox provides: `list_documents()`, `get_section()`, `search_document()`, `llm_query()`
6. Sandbox blocks: `import`, `open`, `eval`, `exec`, shell access
7. Loop continues (up to 15 iterations) until LLM returns `FINAL(answer)`
8. Answer is posted to Discord

## Troubleshooting

**Bot connects but slash commands don't appear**

- Set `DISCORD_GUILD_ID` for instant registration. Without it, global registration takes up to 1 hour.
- Check bot has `applications.commands` scope.

**`/edgar ask` times out**

- Verify LLM endpoint is running: `curl $LLM_BASE_URL/models`
- Try a smaller model if responses are slow.

**PyO3 build errors**

- Ensure `python3` is on PATH and matches the version PyO3 expects.
- On macOS: `brew install python3`
- Check: `python3-config --prefix`

**`cnidarium` storage errors on startup**

- Wipe stale data: `just clean-data`
