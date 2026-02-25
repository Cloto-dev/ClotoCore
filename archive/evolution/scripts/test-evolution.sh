#!/usr/bin/env bash
# Evolution Engine Integration Test Script
# Usage: bash scripts/test-evolution.sh [BASE_URL] [API_KEY]
#
# Requires: running Exiv server, python3, curl, bc
# Tests all evolution trigger types, grace periods, rollbacks, and error handling.

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
NC='\033[0m'

# jq replacement using python3
jq_() {
  python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d$1) if not isinstance(d$1,(int,float,str,bool,type(None))) else d$1)"
}

jq_len() {
  python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d))"
}

auth_args() {
  if [ -n "$API_KEY" ]; then
    echo "-H X-API-Key:${API_KEY}"
  fi
}

evaluate() {
  local cognitive="$1" behavioral="$2" safety="$3" autonomy="$4" meta="$5"
  local snapshot="${6:-}"
  local body
  if [ -n "$snapshot" ]; then
    body=$(printf '{"scores":{"cognitive":%s,"behavioral":%s,"safety":%s,"autonomy":%s,"meta_learning":%s},"snapshot":%s}' \
      "$cognitive" "$behavioral" "$safety" "$autonomy" "$meta" "$snapshot")
  else
    body=$(printf '{"scores":{"cognitive":%s,"behavioral":%s,"safety":%s,"autonomy":%s,"meta_learning":%s}}' \
      "$cognitive" "$behavioral" "$safety" "$autonomy" "$meta")
  fi
  if [ -n "$API_KEY" ]; then
    curl -sf -X POST "${BASE}/evolution/evaluate" \
      -H "Content-Type: application/json" \
      -H "X-API-Key: ${API_KEY}" \
      -d "$body" 2>/dev/null
  else
    curl -sf -X POST "${BASE}/evolution/evaluate" \
      -H "Content-Type: application/json" \
      -d "$body" 2>/dev/null
  fi
}

get_status() {  curl -sf "${BASE}/evolution/status" 2>/dev/null; }
get_generations() { curl -sf "${BASE}/evolution/generations?limit=${1:-50}" 2>/dev/null; }
get_fitness() { curl -sf "${BASE}/evolution/fitness?limit=${1:-100}" 2>/dev/null; }
get_rollbacks() { curl -sf "${BASE}/evolution/rollbacks" 2>/dev/null; }
get_params() { curl -sf "${BASE}/evolution/params" 2>/dev/null; }

# Re-enable agent after safety breach stops it
reenable_agent() {
  local agent_id="${1:-agent.exiv_default}"
  if [ -n "$API_KEY" ]; then
    curl -sf -X POST "${BASE}/agents/${agent_id}/power" \
      -H "Content-Type: application/json" \
      -H "X-API-Key: ${API_KEY}" \
      -d '{"enabled":true}' > /dev/null 2>&1
  else
    curl -sf -X POST "${BASE}/agents/${agent_id}/power" \
      -H "Content-Type: application/json" \
      -d '{"enabled":true}' > /dev/null 2>&1
  fi
}

# Throttled evaluate for loops (avoid rate limiting)
evaluate_throttled() {
  sleep 0.15
  evaluate "$@"
}

assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  TOTAL=$((TOTAL + 1))
  if [ "$expected" = "$actual" ]; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} $desc (expected=$expected)"
  else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} $desc (expected=$expected, got=$actual)"
  fi
}

assert_ge() {
  local desc="$1" min="$2" actual="$3"
  TOTAL=$((TOTAL + 1))
  if [ "$actual" -ge "$min" ] 2>/dev/null; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} $desc (>= $min, got=$actual)"
  else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} $desc (expected >= $min, got=$actual)"
  fi
}

assert_contains() {
  local desc="$1" needle="$2" haystack="$3"
  TOTAL=$((TOTAL + 1))
  if echo "$haystack" | grep -q "$needle"; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} $desc (contains '$needle')"
  else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} $desc (missing '$needle')"
  fi
}

assert_http_error() {
  local desc="$1"
  shift
  TOTAL=$((TOTAL + 1))
  local code
  code=$(curl -s -o /dev/null -w "%{http_code}" "$@" 2>/dev/null)
  if [ "$code" -ge 400 ]; then
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} $desc (HTTP $code)"
  else
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} $desc (expected 4xx/5xx, got HTTP $code)"
  fi
}

# ─────────────────────────────────────────
echo -e "\n${CYAN}═══ Evolution Engine Integration Test ═══${NC}\n"

if ! curl -sf "${BASE}/system/version" > /dev/null 2>&1; then
  echo -e "${RED}ERROR: Server not reachable at ${BASE}${NC}"
  echo "Start the server first: bash start_exiv.sh"
  exit 1
fi
echo -e "${GREEN}Server is running${NC}\n"

# ─────────────────────────────────────────
echo -e "${YELLOW}Scenario 0: Pre-flight checks${NC}"

status=$(get_status)
gen=$(echo "$status" | jq_ "['current_generation']")
echo "  Initial state: generation=$gen"

params=$(get_params)
min_interactions=$(echo "$params" | jq_ "['min_interactions']")
echo "  min_interactions=$min_interactions"

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 1: First evaluation → Gen 1${NC}"

result=$(evaluate 0.5 0.5 1.0 0.4 0.3)
assert_contains "evaluate returns success" '"success"' "$result"
assert_contains "EvolutionGeneration event" 'EvolutionGeneration' "$result"

status=$(get_status)
gen=$(echo "$status" | jq_ "['current_generation']")
assert_eq "generation is 1" "1" "$gen"

gens=$(get_generations)
gen_count=$(echo "$gens" | jq_len)
assert_eq "1 generation record" "1" "$gen_count"

fitness=$(get_fitness)
fit_count=$(echo "$fitness" | jq_len)
assert_eq "1 fitness entry" "1" "$fit_count"

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 2: Small changes → No new generation (debounce)${NC}"

for i in $(seq 1 9); do
  cog=$(python3 -c "print(0.5 + $i * 0.001)")
  evaluate_throttled "$cog" 0.5 1.0 0.4 0.3 > /dev/null
done

status=$(get_status)
gen=$(echo "$status" | jq_ "['current_generation']")
assert_eq "still generation 1" "1" "$gen"

fitness=$(get_fitness)
fit_count=$(echo "$fitness" | jq_len)
assert_eq "10 fitness entries" "10" "$fit_count"

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 3: Large improvement → Evolution trigger (Gen 2)${NC}"

result=$(evaluate 0.85 0.85 1.0 0.4 0.35)
assert_contains "Evolution event" 'Evolution' "$result"

status=$(get_status)
gen=$(echo "$status" | jq_ "['current_generation']")
assert_eq "generation is 2" "2" "$gen"

trend=$(echo "$status" | jq_ "['trend']")
assert_eq "fitness trend is improving" "improving" "$trend"

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 4: Safety Breach → Rollback${NC}"

result=$(evaluate 0.85 0.85 0.0 0.4 0.35)
assert_contains "EvolutionBreach event" 'EvolutionBreach' "$result"

rollbacks=$(get_rollbacks)
rb_count=$(echo "$rollbacks" | jq_len)
assert_ge "at least 1 rollback" "1" "$rb_count"

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 5: Recovery and new baseline${NC}"

# Re-enable agent after safety breach stopped it
reenable_agent
sleep 0.5

for i in $(seq 1 12); do
  evaluate_throttled 0.7 0.7 1.0 0.4 0.3 > /dev/null
done

status=$(get_status)
gen_after_recovery=$(echo "$status" | jq_ "['current_generation']")
echo "  Generation after recovery: $gen_after_recovery"

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 6: Autonomy Upgrade${NC}"

for i in $(seq 1 11); do
  evaluate_throttled 0.7 0.7 1.0 0.4 0.3 > /dev/null
done

gen_before=$(echo "$(get_status)" | jq_ "['current_generation']")

result=$(evaluate_throttled 0.7 0.7 1.0 0.8 0.3)
gen_after=$(echo "$(get_status)" | jq_ "['current_generation']")

if [ "$gen_after" -gt "$gen_before" ]; then
  assert_contains "AutonomyUpgrade trigger" 'Autonomy' "$result"
else
  echo -e "  ${YELLOW}SKIP${NC} AutonomyUpgrade (may have been overridden by other trigger)"
fi

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 7: CapabilityGain (snapshot with new plugin)${NC}"

for i in $(seq 1 11); do
  evaluate_throttled 0.7 0.7 1.0 0.6 0.3 > /dev/null
done

gen_before=$(echo "$(get_status)" | jq_ "['current_generation']")

snapshot='{"active_plugins":["mind.deepseek","vision.new_plugin"],"plugin_capabilities":{"mind.deepseek":["Reasoning"],"vision.new_plugin":["Vision"]},"personality_hash":"test","strategy_params":{}}'
result=$(evaluate_throttled 0.7 0.7 1.0 0.6 0.3 "$snapshot")

gen_after=$(echo "$(get_status)" | jq_ "['current_generation']")
if [ "$gen_after" -gt "$gen_before" ]; then
  assert_contains "CapabilityGain trigger" 'Capability' "$result"
else
  echo -e "  ${YELLOW}INFO${NC} No generation created (debounce or trigger priority)"
fi

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 8: Error handling${NC}"

if [ -n "$API_KEY" ]; then
  AUTH_H="-H X-API-Key:${API_KEY}"
else
  AUTH_H=""
fi

assert_http_error "Reject cognitive > 1.0" \
  -X POST "${BASE}/evolution/evaluate" \
  -H "Content-Type: application/json" $AUTH_H \
  -d '{"scores":{"cognitive":1.5,"behavioral":0.5,"safety":1.0,"autonomy":0.4,"meta_learning":0.3}}'

assert_http_error "Reject negative behavioral" \
  -X POST "${BASE}/evolution/evaluate" \
  -H "Content-Type: application/json" $AUTH_H \
  -d '{"scores":{"cognitive":0.5,"behavioral":-0.1,"safety":1.0,"autonomy":0.4,"meta_learning":0.3}}'

assert_http_error "Reject missing scores" \
  -X POST "${BASE}/evolution/evaluate" \
  -H "Content-Type: application/json" $AUTH_H \
  -d '{"foo":"bar"}'

# ─────────────────────────────────────────
echo -e "\n${YELLOW}Scenario 9: API endpoint consistency${NC}"

status=$(get_status)
assert_contains "status has current_generation" 'current_generation' "$status"
assert_contains "status has trend" 'trend' "$status"
assert_contains "status has fitness" '"fitness"' "$status"

gens=$(get_generations 5)
assert_contains "generations is array" '\[' "$gens"

fitness=$(get_fitness 10)
assert_contains "fitness is array" '\[' "$fitness"

params=$(get_params)
assert_contains "params has alpha" 'alpha' "$params"
assert_contains "params has weights" 'weights' "$params"

rollbacks=$(get_rollbacks)
assert_contains "rollbacks is array" '\[' "$rollbacks"

# ─────────────────────────────────────────
echo -e "\n${CYAN}═══ Results ═══${NC}"
echo -e "  Total:  $TOTAL"
echo -e "  ${GREEN}Passed: $PASS${NC}"
if [ "$FAIL" -gt 0 ]; then
  echo -e "  ${RED}Failed: $FAIL${NC}"
  exit 1
else
  echo -e "  Failed: 0"
  echo -e "\n${GREEN}All tests passed!${NC}"
fi
