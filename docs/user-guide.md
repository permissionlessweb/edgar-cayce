# User Guide

## Asking Questions

```
/edgar ask topic:akash-docs question:How do I deploy a container?
```

Edgar reads the ingested documents, searches for relevant content, and returns an answer with source links.

## Browsing Sources

```
/edgar sources
```

Shows all ingested documents grouped by topic. Use the topic names here when asking questions.

## Threads

```
/edgar thread name:deployment-help
```

Creates a dedicated thread for your conversation.

## Tips

- Match the `topic` to a label from `/edgar sources`
- Ask specific questions — "How does X work?" beats "Tell me about X"
- Source links at the bottom of answers point to the original files
