#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════
# Exiv Live Auto-Evaluation Test
# ══════════════════════════════════════════════════════════════
# Sends real chat messages → LLM responds → FitnessCollector auto-evaluates
# Monitors evolution state changes in real-time.
#
# Usage: bash scripts/test-live-eval.sh [BASE_URL] [API_KEY]

set -euo pipefail

BASE="${1:-http://127.0.0.1:8081/api}"
API_KEY="${2:-}"
PASS=0
FAIL=0
TOTAL=0

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

auth_header() {
  if [ -n "$API_KEY" ]; then echo "-H X-API-Key:${API_KEY}"; fi
}

jq_() {
  python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d$1) if not isinstance(d$1,(int,float,str,bool,type(None))) else d$1)" 2>/dev/null
}

send_chat() {
  local msg_id="$1" content="$2"
  curl -sf -X POST "${BASE}/chat" \
    -H "Content-Type: application/json" \
    -H "X-API-Key: ${API_KEY}" \
    -d "$(python3 -c "
import json, datetime
print(json.dumps({
    'id': '$msg_id',
    'source': {'type': 'User', 'id': 'live-test', 'name': 'LiveTest'},
    'content': '$content',
    'timestamp': datetime.datetime.utcnow().isoformat() + 'Z',
    'metadata': {}
}))
")" > /dev/null 2>&1
}

get_status() { curl -sf "${BASE}/evolution/status" 2>/dev/null; }

get_latest_fitness() {
  curl -sf "${BASE}/evolution/fitness?limit=1" 2>/dev/null | python3 -c "
import sys, json
entries = json.load(sys.stdin)
if entries:
    e = entries[-1]
    s = e['scores']
    print(f'cog={s[\"cognitive\"]:.3f} beh={s[\"behavioral\"]:.3f} saf={s[\"safety\"]:.1f} aut={s[\"autonomy\"]:.3f} meta={s[\"meta_learning\"]:.3f}')
else:
    print('(no data)')
" 2>/dev/null
}

assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  TOTAL=$((TOTAL + 1))
  if [ "$expected" = "$actual" ]; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} $desc"
  else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} $desc (expected=$expected, got=$actual)"
  fi
}

assert_gt() {
  local desc="$1" threshold="$2" actual="$3"
  TOTAL=$((TOTAL + 1))
  if python3 -c "exit(0 if $actual > $threshold else 1)" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} $desc ($actual > $threshold)"
  else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} $desc (expected > $threshold, got=$actual)"
  fi
}

assert_range() {
  local desc="$1" lo="$2" hi="$3" actual="$4"
  TOTAL=$((TOTAL + 1))
  if python3 -c "exit(0 if $lo <= $actual <= $hi else 1)" 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} $desc ($actual in [$lo, $hi])"
  else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} $desc (expected [$lo, $hi], got=$actual)"
  fi
}

# ══════════════════════════════════════════════════════════════

echo -e "\n${CYAN}═══ Exiv Live Auto-Evaluation Test ═══${NC}\n"

if ! curl -sf "${BASE}/system/version" > /dev/null 2>&1; then
  echo -e "${RED}ERROR: Server not reachable${NC}"
  exit 1
fi
echo -e "${GREEN}Server running${NC}\n"

# Record initial state
initial_status=$(get_status)
initial_interactions=$(echo "$initial_status" | jq_ "['interaction_count']")
initial_gen=$(echo "$initial_status" | jq_ "['current_generation']")
echo -e "  Initial: gen=$initial_gen interactions=$initial_interactions\n"

# ─── Test 1: Single chat → ThoughtResponse → auto-eval ───
echo -e "${YELLOW}Test 1: Single chat triggers auto-evaluation${NC}"

send_chat "live-001" "What is the capital of France?"
echo "  Sent chat message, waiting for LLM response..."
sleep 12

status=$(get_status)
new_interactions=$(echo "$status" | jq_ "['interaction_count']")
interactions_delta=$((new_interactions - initial_interactions))

assert_gt "Interaction count increased" 0 "$interactions_delta"

latest=$(get_latest_fitness)
echo -e "  Latest fitness: $latest"

# Auto-eval scores should show defaults for plugin axes
latest_scores=$(curl -sf "${BASE}/evolution/fitness?limit=1" 2>/dev/null)
cog=$(echo "$latest_scores" | python3 -c "import sys,json; print(json.load(sys.stdin)[-1]['scores']['cognitive'])" 2>/dev/null || echo "?")
meta=$(echo "$latest_scores" | python3 -c "import sys,json; print(json.load(sys.stdin)[-1]['scores']['meta_learning'])" 2>/dev/null || echo "?")
saf=$(echo "$latest_scores" | python3 -c "import sys,json; print(json.load(sys.stdin)[-1]['scores']['safety'])" 2>/dev/null || echo "?")
beh=$(echo "$latest_scores" | python3 -c "import sys,json; print(json.load(sys.stdin)[-1]['scores']['behavioral'])" 2>/dev/null || echo "?")

assert_eq "Cognitive is default (0.5, no plugin)" "0.5" "$cog"
assert_eq "Meta-learning is default (0.5, no plugin)" "0.5" "$meta"
assert_eq "Safety is 1.0 (no violations)" "1.0" "$saf"
assert_gt "Behavioral computed from events (> 0)" 0 "$beh"

echo ""

# ─── Test 2: Multiple chats → score accumulation ───
echo -e "${YELLOW}Test 2: Multiple chats accumulate metrics${NC}"

questions=(
  "What is machine learning?"
  "Explain quantum computing briefly."
  "How does photosynthesis work?"
  "What is the speed of light?"
  "Name three programming languages."
)

for i in "${!questions[@]}"; do
  send_chat "live-batch-$((i+1))" "${questions[$i]}"
  echo -e "  📤 Sent: ${questions[$i]}"
  sleep 8
done

echo "  Waiting for all responses..."
sleep 5

status=$(get_status)
batch_interactions=$(echo "$status" | jq_ "['interaction_count']")
batch_delta=$((batch_interactions - new_interactions))

assert_gt "5 more interactions tracked" 3 "$batch_delta"

# Check behavioral score evolved (should improve with more successful responses)
latest_beh=$(curl -sf "${BASE}/evolution/fitness?limit=1" 2>/dev/null | \
  python3 -c "import sys,json; print(json.load(sys.stdin)[-1]['scores']['behavioral'])" 2>/dev/null || echo "0")
assert_gt "Behavioral score positive" 0 "$latest_beh"

echo ""

# ─── Test 3: Evolution state consistency ───
echo -e "${YELLOW}Test 3: Evolution state consistency${NC}"

status=$(get_status)
gen=$(echo "$status" | jq_ "['current_generation']")
fitness=$(echo "$status" | jq_ "['fitness']")
trend=$(echo "$status" | jq_ "['trend']")

echo -e "  Current: gen=$gen fitness=$fitness trend=$trend"

assert_gt "Generation >= initial" "$((initial_gen - 1))" "$gen"
assert_gt "Fitness is positive" 0 "$fitness"

# Verify fitness timeline has entries from auto-eval
timeline_count=$(curl -sf "${BASE}/evolution/fitness?limit=1000" 2>/dev/null | \
  python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "0")
assert_gt "Fitness timeline has entries" "$initial_interactions" "$timeline_count"

echo ""

# ─── Test 4: Dashboard API endpoints all working ───
echo -e "${YELLOW}Test 4: Dashboard API health check${NC}"

for endpoint in status "generations?limit=5" "fitness?limit=5" rollbacks params; do
  code=$(curl -s -o /dev/null -w "%{http_code}" "${BASE}/evolution/${endpoint}")
  TOTAL=$((TOTAL + 1))
  if [ "$code" = "200" ]; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} /evolution/$endpoint (HTTP $code)"
  else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} /evolution/$endpoint (HTTP $code)"
  fi
done

echo ""

# ─── Results ───
echo -e "${CYAN}═══ Results ═══${NC}"
echo -e "  Total:  $TOTAL"
echo -e "  ${GREEN}Passed: $PASS${NC}"
if [ "$FAIL" -gt 0 ]; then
  echo -e "  ${RED}Failed: $FAIL${NC}"
  exit 1
else
  echo -e "  Failed: 0"
  echo -e "\n${GREEN}All live auto-evaluation tests passed!${NC}"
fi
echo ""
