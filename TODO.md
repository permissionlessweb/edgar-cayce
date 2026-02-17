# TODO

## SYSTEM PROMPT IMPROVEMENTS

- define:
  - how documents are ingested
  - how documents are labeled
  - structured output formats

### citing sources + document url routing

we must be able to attribute document sources with public urls. right now we are only aware of file/context location rleative to github repo. we can provide a public-url prompt admins are able to set when ingesting documents that say:

```
"all infomration found in docs/ folder can be routed/attributed to the url <https://akash.network/docs>"
```

alternatively,we cna add support for a headless browser/ web search tool for fetching data from public documetnation url (set by admins) for crosslinking and refernceing source location,.

- improve rlm tooling available
  - integrate vector embeddings
  - sub-rlm route for finding previous questions and workflows
- record q/a + feeback on quality of response
- benchmarks, test suite

## DISCORD UX/UI Improvements

### PRIORITY

- alwasy display question + prompt public (right now its obfuscated from tool call)
-

### IDEAS

- discord bot reacts to questions coming into chat, rather than prompted "is this a question related to one of the documentations i have?", "has this user paid a premium to have their questions answered"
- create & manage dedicated threads: create threads for specific questions, answer in threads, but questiosn can be asked in any channels (ping user with response linking to specific thread with answer). use existing thread answers for additional context
- upvoting downvoting: allow users(specific roles) to provide feedback on the answers being provided
