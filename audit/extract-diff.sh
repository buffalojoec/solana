#!/usr/bin/env bash
#
# Extract diff and stat summary for a squash-merged PR.
#
# Usage: ./audit/extract-diff.sh <pr_number>
#
# Resolves the squash commit by matching "(#<number>)" in commit messages.
#
# Outputs:
#   audit/diffs/PR_<number>.stat  — file-level change summary
#   audit/diffs/PR_<number>.diff  — full unified diff

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <pr_number>" >&2
    exit 1
fi

PR="$1"
DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="$DIR/diffs"

mkdir -p "$OUT"

# Find the squash commit by matching the "(#NNNN)" suffix in the subject line.
# Use --all to search all branches. Filter --after 2024 to exclude old Solana
# monorepo commits that share PR numbers with Agave.
MATCHES=$(git log --all --after="2023-12-01" --grep="(#${PR})$" --format='%H %s')

if [[ -z "$MATCHES" ]]; then
    echo "Error: no commit found matching (#${PR})" >&2
    exit 1
fi

COUNT=$(echo "$MATCHES" | wc -l)
if [[ "$COUNT" -gt 1 ]]; then
    echo "Error: multiple commits match (#${PR}):" >&2
    echo "$MATCHES" >&2
    echo "" >&2
    echo "Resolve manually and use: git diff <sha>^..<sha>" >&2
    exit 1
fi

COMMIT=$(echo "$MATCHES" | awk '{print $1}')
TITLE=$(git log -1 --format='%s' "$COMMIT")

STAT_FILE="$OUT/PR_${PR}.stat"
DIFF_FILE="$OUT/PR_${PR}.diff"

# Write stat summary
echo "# PR #${PR} — ${TITLE}" > "$STAT_FILE"
echo "# Commit: ${COMMIT}" >> "$STAT_FILE"
echo "# Date: $(git log -1 --format='%ai' "$COMMIT")" >> "$STAT_FILE"
echo "#" >> "$STAT_FILE"
git diff --stat "$COMMIT"^.."$COMMIT" >> "$STAT_FILE"

# Write full diff
git diff "$COMMIT"^.."$COMMIT" > "$DIFF_FILE"

LINES=$(wc -l < "$DIFF_FILE")
echo "PR #${PR} [${COMMIT:0:10}]: ${TITLE}"
echo "  stat -> ${STAT_FILE} | diff -> ${DIFF_FILE} (${LINES} lines)"
