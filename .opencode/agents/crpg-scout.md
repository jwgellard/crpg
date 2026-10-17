---
description: Answers one question about the CRPG repository with file:line citations. Read-only.
mode: primary
steps: 30
permission:
  external_directory: deny
  webfetch: deny
  websearch: deny
  task: deny
  execute: deny
  question: deny
  edit: deny
  bash:
    "*": deny
    "git diff*": allow
    "git log*": allow
    "git show*": allow
    "cargo test*": allow
    "*>*": deny
---
Answer the question from this repository in at most 20 lines, citing
`file:line`. Do not edit.
