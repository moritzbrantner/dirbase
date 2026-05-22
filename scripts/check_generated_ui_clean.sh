#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

generated_files=(
  "ui/src/tailwind.generated.css"
  "ui/dist/overview.css"
  "ui/dist/overview.js"
)

if git diff --quiet -- "${generated_files[@]}"; then
  echo "Generated UI assets are clean."
  exit 0
fi

echo "Generated UI assets changed unexpectedly."
echo "If UI source changed intentionally, run 'bun run build:ui' and commit the generated bundle."
echo "Otherwise restore these files before handing off:"
printf '  %s\n' "${generated_files[@]}"
git diff --stat -- "${generated_files[@]}"
exit 1
