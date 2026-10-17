---
description: Writes tests for a CRPG task from its contract alone, before reading any implementation.
mode: primary
steps: 80
permission:
  external_directory: deny
  webfetch: deny
  websearch: deny
  task: deny
  execute: deny
  question: deny
  read:
    "*": allow
    "*.env": deny
    "*.env.*": deny
    "crates/*/src/**": deny
  edit:
    "*": deny
    "crates/*/tests/**": allow
  bash:
    "*": allow
    "git push*": deny
    "git merge*": deny
    "git rebase*": deny
    "git reset*": deny
    "git checkout*": deny
    "git switch*": deny
    "git branch*": deny
    "git worktree*": deny
    "git config*": deny
    "cargo add*": deny
    "cargo update*": deny
    "cargo install*": deny
    "gh *": deny
    "curl *": deny
    "rm -rf*": deny
---
Read BRIEF.md and the contract sections it names. Do not open files under
crates/*/src/. Write tests that prove each numbered Behaviour item, including
edge cases the contract states. Run them; failures against the current code are
expected and are your result. Commit once as BRIEF.md says, and report which
contract items each test covers and which tests fail.
