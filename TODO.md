# TODO

## SYSTEM PROMPT IMPROVEMENTS

- define:
  - how documents are ingested
  - how documents are labeled
  - structured output formats

- web seach tool: enable web search on per-document basis. only allow specific websites registred to be included in web tool query

## RLM TOOLING IMPROVEMENTS

- integrate vector embeddings
- sub-rlm route for finding previous questions and workflows

## STORAGE/DB IMPROVEMENTS

- record q/a + feeback on quality of response
- benchmarks, test suite
- compression workflows
- exporting workflow: export database logs to s3 bucket,portable for out of band classification

## DISCORD UX/UI Improvements

### PRIORITY

- alwasy display question + prompt public (right now its obfuscated from tool call)
- respond with question revieved, display question, and then display pending until loop is complete, update msg with completed answer

### IDEAS

- discord bot reacts to questions coming into chat, rather than prompted "is this a question related to one of the documentations i have?", "has this user paid a premium to have their questions answered"
- create & manage dedicated threads: create threads for specific questions, answer in threads, but questiosn can be asked in any channels (ping user with response linking to specific thread with answer). use existing thread answers for additional context
- upvoting downvoting: allow users(specific roles) to provide feedback on the answers being provided
