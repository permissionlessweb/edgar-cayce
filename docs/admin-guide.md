# Admin Guide

## Setup

Set these environment variables:

```
DISCORD_TOKEN=your-bot-token
DISCORD_GUILD_ID=123456789          # optional, speeds up command registration
ADMIN_USER_IDS=111111111,222222222  # comma-separated Discord user IDs
ADMIN_ROLE_IDS=333333333            # optional, comma-separated role IDs
```

LLM provider (one of):

```
OPENAI_API_KEY=sk-...
ANTHROPIC_API_KEY=sk-ant-...
OPENROUTER_API_KEY=sk-or-...
```

## Ingesting Documents

GitHub repo:

```
/edgar ingest url:https://github.com/akash-network/docs label:akash-docs
```

With citation context (so answers link back to source files):

```
/edgar ingest url:https://github.com/org/repo label:my-topic url_context:Source files viewable at https://github.com/org/repo/blob/main/{filepath}
```

Web page:

```
/edgar ingest url:https://example.com/docs/guide label:my-topic doc_type:web
```

## Tuning the Reasoning Engine

View current settings:

```
/edgar config rlm
```

Adjust:

```
/edgar config rlm max_iterations:20 min_code_executions:4 parallel_loops:3
```

| Parameter | Default | What it controls |
|-----------|---------|------------------|
| `max_iterations` | 15 | REPL loop ceiling per question |
| `min_code_executions` | 3 | Minimum doc reads before answering |
| `min_answer_len` | 150 | Reject answers shorter than this |
| `parallel_loops` | 2 | Sub-investigations for complex questions |

## Managing Admin Roles

```
/edgar config roles-list
/edgar config roles-add role:@Moderators
/edgar config roles-remove role:@Moderators
```

## Debug Mode

```
/edgar ask topic:akash-docs question:How does bidding work? debug:true
```

Returns the answer plus raw evidence the engine collected. Admin-only.
