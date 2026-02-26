# TODO

## SYSTEM PROMPT IMPROVEMENTS

- define:
  - how documents are ingested
  - how documents are labeled
  - structured output formats

- web seach tool: enable web search on per-document basis. only allow specific websites registred to be included in web tool query
- identify pictures in questions, parse picture via llm, use in rlm loop

## RLM TOOLING IMPROVEMENTS

- integrate vector embeddings
- sub-rlm route for finding previous questions and workflows:
  - check for exiting questions -> grade similarity + include in context loop -> continue ()
  - qmd + rag embedding

## STORAGE/DB IMPROVEMENTS

- record q/a + feeback on quality of response
- benchmarks, test suite
- compression workflows
- exporting workflow: export database logs to s3 bucket,portable for out of band classification

## DISCORD UX/UI Improvements

### PRIORITY

- alwasy display question + prompt public (right now its obfuscated from tool call)
- create & manage dedicated threads: create threads for specific questions, answer in threads, but questiosn can be asked in any channels (ping user with response linking to specific thread with answer). use existing thread answers for additional context

### IDEAS

- discord bot reacts to questions coming into chat, rather than prompted. uses small model iterative rlm loop "is this a question related to one of the documentations i have?", "has this user paid a premium to have their questions answered?", "have I answered this question already?"
- upvoting downvoting: allow users(specific roles) to provide feedback on the answers being provided
  - tell bot: "this question has been answered correctly with this answer

## TRANSPARENCY

- provide users with a view of the system prompt being used
- proivde user with the url_context prompt being used

## BENCHMARKING

- token-count per iteration, tokens-per-second (input & output)
- system prompt token count
- benchmarking suite (common questions, reusable prompts)
- flamegraph usage per prompt
