# Repository Rules

- Apply clean code principles to every change: prefer clear names, small focused functions, low duplication, explicit data flow, and existing project patterns over new abstractions.
- Keep refactors behavior-preserving unless the user explicitly requests behavior changes.
- When duplicate logic appears across Rust, UI, or JS modules, extract a shared helper in the nearest appropriate module instead of editing copies independently.
- Run the relevant tests after changes. For broad repository changes, use `scripts/run_repo_tests.sh`.
