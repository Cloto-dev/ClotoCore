#!/usr/bin/env bash
# sync-issues-to-github.sh — Sync issue registry with GitHub Issues
#
# Called by the issue-sync GitHub Actions workflow on PR merge.
# Two operations:
#   1. CREATE: New registry entries without github_issue → create GitHub Issue,
#              write back the issue number to the registry, and commit.
#   2. CLOSE:  Entries that changed status to "fixed" with github_issue
#              → close the linked GitHub Issue with a comment.
#
# Environment variables (set by GitHub Actions):
#   BASE_SHA            — Base commit SHA (before PR merge)
#   PR_NUMBER           — The merged PR number
#   GH_TOKEN            — GitHub token for gh CLI
#   GITHUB_REPOSITORY   — owner/repo
#
# Usage: bash scripts/sync-issues-to-github.sh

set -euo pipefail

CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

echo -e "${CYAN}=== Issue Registry ↔ GitHub Sync ===${NC}"

: "${BASE_SHA:?BASE_SHA is required}"
: "${PR_NUMBER:?PR_NUMBER is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"

REGISTRY="qa/issue-registry.json"

# Detect Python
PYTHON_CMD="python3"
if ! command -v "$PYTHON_CMD" &>/dev/null; then
    PYTHON_CMD="python"
fi

# Ensure labels exist (idempotent — gh label create is a no-op if label already exists)
gh label create "auto-created" --description "Auto-created by issue registry sync" --color "c5def5" --repo "$GITHUB_REPOSITORY" 2>/dev/null || true
gh label create "severity: critical" --color "b60205" --repo "$GITHUB_REPOSITORY" 2>/dev/null || true
gh label create "severity: high" --color "d93f0b" --repo "$GITHUB_REPOSITORY" 2>/dev/null || true
gh label create "severity: medium" --color "fbca04" --repo "$GITHUB_REPOSITORY" 2>/dev/null || true
gh label create "severity: low" --color "0e8a16" --repo "$GITHUB_REPOSITORY" 2>/dev/null || true

# Get old registry from base commit
OLD_REGISTRY_FILE=$(mktemp)
git show "${BASE_SHA}:${REGISTRY}" > "$OLD_REGISTRY_FILE" 2>/dev/null || echo '{"issues":[]}' > "$OLD_REGISTRY_FILE"

SEVERITY_LABELS='{"CRITICAL":"severity: critical","HIGH":"severity: high","MEDIUM":"severity: medium","LOW":"severity: low"}'

# ──────────────────────────────────────────────
# Phase 1: Create GitHub Issues for new entries
# ──────────────────────────────────────────────
echo -e "\n${CYAN}Phase 1: Create GitHub Issues${NC}"

NEW_ENTRIES=$($PYTHON_CMD -c "
import json

with open('$OLD_REGISTRY_FILE') as f:
    old_data = json.load(f)
with open('$REGISTRY') as f:
    new_data = json.load(f)

old_ids = {e['id'] for e in old_data.get('issues', [])}

for entry in new_data.get('issues', []):
    eid = entry['id']
    if eid in old_ids:
        continue
    if entry.get('github_issue'):
        continue
    if entry.get('status') in ('wontfix', 'obsolete'):
        continue
    # New entry without github_issue — needs creation
    # Output: id|severity|summary|file|status
    print('|'.join([
        eid,
        entry.get('severity', 'MEDIUM'),
        entry.get('summary', ''),
        entry.get('file', ''),
        entry.get('status', 'open'),
    ]))
" || true)

created=0
# Associative array to track created issues: bug_id → github_issue_number
declare -A CREATED_MAP

if [[ -n "$NEW_ENTRIES" ]]; then
    while IFS='|' read -r bug_id severity summary file status; do
        echo -e "  Creating GitHub Issue for ${bug_id} (${severity}): ${summary}"

        label="bug"
        sev_label=$($PYTHON_CMD -c "import json; print(json.loads('$SEVERITY_LABELS').get('$severity', 'severity: medium'))")

        body="> [!NOTE]
> This issue was **automatically created** by the issue registry sync workflow.
> Source: [\`qa/issue-registry.json\`](https://github.com/${GITHUB_REPOSITORY}/blob/master/qa/issue-registry.json) — PR #${PR_NUMBER}

## Registry Entry

| Field | Value |
|-------|-------|
| **ID** | \`${bug_id}\` |
| **Severity** | ${severity} |
| **File** | \`${file}\` |
| **Status** | ${status} |

## Description

${summary}"

        issue_url=$(gh issue create \
            --title "[${bug_id}] ${summary}" \
            --body "$body" \
            --label "$label" \
            --label "$sev_label" \
            --label "auto-created" \
            --repo "$GITHUB_REPOSITORY" 2>/dev/null) || {
            echo -e "  ${YELLOW}Warning: Failed to create issue for ${bug_id}${NC}"
            continue
        }

        # Extract issue number from URL (https://github.com/owner/repo/issues/123)
        issue_num=$(echo "$issue_url" | grep -oE '[0-9]+$')
        if [[ -n "$issue_num" ]]; then
            CREATED_MAP["$bug_id"]="$issue_num"
            echo -e "  ${GREEN}Created #${issue_num}${NC}"
            created=$((created + 1))
        fi
    done <<< "$NEW_ENTRIES"
else
    echo -e "  ${YELLOW}No new entries to create.${NC}"
fi

# Write back github_issue numbers to registry
if [[ ${#CREATED_MAP[@]} -gt 0 ]]; then
    echo -e "\n  Writing back github_issue numbers to registry..."

    # Build a JSON map of bug_id → issue_number
    MAP_JSON="{"
    first=true
    for bug_id in "${!CREATED_MAP[@]}"; do
        if [[ "$first" == "true" ]]; then
            first=false
        else
            MAP_JSON+=","
        fi
        MAP_JSON+="\"${bug_id}\":${CREATED_MAP[$bug_id]}"
    done
    MAP_JSON+="}"

    $PYTHON_CMD -c "
import json

with open('$REGISTRY', 'r') as f:
    data = json.load(f)

created = json.loads('$MAP_JSON')

for entry in data.get('issues', []):
    if entry['id'] in created:
        entry['github_issue'] = created[entry['id']]

with open('$REGISTRY', 'w') as f:
    json.dump(data, f, indent=2, ensure_ascii=False)
    f.write('\n')
"

    git config user.name "github-actions[bot]"
    git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
    git add "$REGISTRY"
    git commit -m "chore: link registry entries to GitHub Issues (#${PR_NUMBER})

Auto-linked ${#CREATED_MAP[@]} registry entry/entries to GitHub Issues.

Co-Authored-By: github-actions[bot] <41898282+github-actions[bot]@users.noreply.github.com>"
    git push
    echo -e "  ${GREEN}Committed github_issue links.${NC}"
fi

# ──────────────────────────────────────────────
# Phase 2: Close GitHub Issues for fixed entries
# ──────────────────────────────────────────────
echo -e "\n${CYAN}Phase 2: Close fixed GitHub Issues${NC}"

ISSUES_TO_CLOSE=$($PYTHON_CMD -c "
import json

with open('$OLD_REGISTRY_FILE') as f:
    old_data = json.load(f)
with open('$REGISTRY') as f:
    new_data = json.load(f)

old_map = {e['id']: e for e in old_data.get('issues', [])}

for entry in new_data.get('issues', []):
    gh = entry.get('github_issue')
    if not gh:
        continue
    if entry.get('status') != 'fixed':
        continue
    old_entry = old_map.get(entry['id'], {})
    if old_entry.get('status') == 'fixed':
        continue
    print(f\"{gh}|{entry['id']}|{entry.get('summary', '')}\")
" || true)

rm -f "$OLD_REGISTRY_FILE"

closed=0
if [[ -n "$ISSUES_TO_CLOSE" ]]; then
    while IFS='|' read -r issue_num bug_id summary; do
        echo -e "  Closing #${issue_num} (${bug_id}: ${summary})"
        gh issue close "$issue_num" \
            --comment "Resolved via PR #${PR_NUMBER} — \`${bug_id}\` verified as fixed by [issue registry](https://github.com/${GITHUB_REPOSITORY}/blob/master/qa/issue-registry.json)." \
            --repo "$GITHUB_REPOSITORY" \
        && closed=$((closed + 1)) \
        || echo -e "  ${YELLOW}Warning: Failed to close #${issue_num}${NC}"
    done <<< "$ISSUES_TO_CLOSE"
else
    echo -e "  ${YELLOW}No issues to close.${NC}"
fi

# ──────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────
echo ""
echo -e "${CYAN}=== Summary ===${NC}"
echo -e "Created: ${GREEN}${created}${NC}"
echo -e "Closed:  ${GREEN}${closed}${NC}"
