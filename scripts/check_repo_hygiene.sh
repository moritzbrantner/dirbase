#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

print_section() {
  printf '\n== %s ==\n' "$1"
}

print_section "Git status"
git status --short --branch

print_section "Tracked changes"
if git diff --quiet && git diff --cached --quiet; then
  echo "No tracked changes."
else
  if ! git diff --cached --quiet; then
    echo "Staged:"
    git diff --cached --name-status
  fi
  if ! git diff --quiet; then
    echo "Unstaged:"
    git diff --name-status
  fi
fi

print_section "Untracked files"
untracked_files="$(git ls-files --others --exclude-standard)"
if [[ -z "$untracked_files" ]]; then
  echo "No untracked files."
else
  echo "$untracked_files"
fi

print_section "Upstream"
if upstream="$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null)"; then
  echo "Upstream: $upstream"
  read -r behind ahead < <(git rev-list --left-right --count "$upstream"...HEAD)
  echo "Behind: $behind"
  echo "Ahead: $ahead"
else
  echo "No upstream branch configured."
fi

print_section "Generated directories tracked by Git"
generated_paths=(
  "target"
  "node_modules"
  "js/node_modules"
  "js/dist"
  "js/bin"
  "ui/node_modules"
  "ui/coverage"
  "ui/test-results"
  "ui/playwright-report"
  "benchmarks/.work"
  "benchmarks/json-server/node_modules"
  "benchmarks/json-graphql-server/node_modules"
)

tracked_generated=0
for path in "${generated_paths[@]}"; do
  if git ls-files -- "$path" | grep -q .; then
    echo "$path"
    tracked_generated=1
  fi
done

if [[ "$tracked_generated" -eq 0 ]]; then
  echo "No accidentally tracked generated directories detected."
fi

print_section "Intentional generated artifacts"
echo "ui/dist/overview.css and ui/dist/overview.js are checked in for Cargo builds."
echo "benchmarks/results/suite-logs-* contains historical benchmark logs and is ignored for new runs."

print_section "Local-only ignore coverage"
local_only_paths=(
  ".env"
  ".env.local"
  "requests.log"
  "data/local.json"
  "target/debug/dirbase"
  "ui/coverage/index.html"
  "ui/test-results/results.json"
  "ui/playwright-report/index.html"
  "js/dist/index.js"
  "js/bin/linux-x64/dirbase"
  "benchmarks/.work/dirbase.log"
  "benchmarks/results/benchmark-summary-local.json"
)

missing_ignores=0
for path in "${local_only_paths[@]}"; do
  if git check-ignore --quiet --no-index "$path"; then
    echo "ignored: $path"
  else
    echo "not ignored: $path"
    missing_ignores=1
  fi
done

if [[ "$missing_ignores" -eq 0 ]]; then
  echo "All checked local-only paths are ignored."
else
  echo "Review the not ignored paths before creating local artifacts there."
fi
