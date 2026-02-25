#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════
# Exiv Evolution Engine — 30-Minute Benchmark Test
# ══════════════════════════════════════════════════════════════
#
# Simulates realistic AI operation patterns over 30 minutes:
#   Phase 1 (0-5m):   Warm-up — steady baseline interactions
#   Phase 2 (5-12m):  Growth — gradual improvement in scores
#   Phase 3 (12-16m): Stress — inject safety breach + recovery
#   Phase 4 (16-22m): Mastery — high performance, capability gain
#   Phase 5 (22-27m): Volatility — score fluctuations, test debounce
#   Phase 6 (27-30m): Final — stability check, summary
#
# Usage: bash scripts/benchmark-30min.sh [BASE_URL] [API_KEY]
# Requires: running Exiv server, python3, curl, bc

set -euo pipefail

BASE="${1:-http://127.0.0.1:8081/api}"
API_KEY="${2:-}"
DURATION=1800  # 30 minutes in seconds
INTERVAL=10    # seconds between evaluations
START_TIME=$(date +%s)

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
NC='\033[0m'

# Stats
EVAL_COUNT=0
GEN_EVENTS=0
BREACH_EVENTS=0
ROLLBACK_EVENTS=0
WARNING_EVENTS=0
CAPABILITY_EVENTS=0
ERRORS=0

# ── Helper functions ──

jq_() {
  python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d$1) if not isinstance(d$1,(int,float,str,bool,type(None))) else d$1)" 2>/dev/null
}

elapsed() {
  echo $(( $(date +%s) - START_TIME ))
}

elapsed_fmt() {
  local e=$(elapsed)
  printf "%02d:%02d" $((e / 60)) $((e % 60))
}

phase_name() {
  local e=$(elapsed)
  if   [ "$e" -lt 300 ];  then echo "Phase 1: Warm-up"
  elif [ "$e" -lt 720 ];  then echo "Phase 2: Growth"
  elif [ "$e" -lt 960 ];  then echo "Phase 3: Stress"
  elif [ "$e" -lt 1320 ]; then echo "Phase 4: Mastery"
  elif [ "$e" -lt 1620 ]; then echo "Phase 5: Volatility"
  else                          echo "Phase 6: Final"
  fi
}

auth_header() {
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
  local result
  if [ -n "$API_KEY" ]; then
    result=$(curl -sf -X POST "${BASE}/evolution/evaluate" \
      -H "Content-Type: application/json" \
      -H "X-API-Key: ${API_KEY}" \
      -d "$body" 2>/dev/null) || { ERRORS=$((ERRORS+1)); return 1; }
  else
    result=$(curl -sf -X POST "${BASE}/evolution/evaluate" \
      -H "Content-Type: application/json" \
      -d "$body" 2>/dev/null) || { ERRORS=$((ERRORS+1)); return 1; }
  fi
  EVAL_COUNT=$((EVAL_COUNT+1))

  # Count events
  if echo "$result" | grep -q "EvolutionGeneration"; then
    GEN_EVENTS=$((GEN_EVENTS+1))
  fi
  if echo "$result" | grep -q "EvolutionBreach"; then
    BREACH_EVENTS=$((BREACH_EVENTS+1))
  fi
  if echo "$result" | grep -q "EvolutionRollback"; then
    ROLLBACK_EVENTS=$((ROLLBACK_EVENTS+1))
  fi
  if echo "$result" | grep -q "EvolutionWarning"; then
    WARNING_EVENTS=$((WARNING_EVENTS+1))
  fi
  if echo "$result" | grep -q "EvolutionCapability"; then
    CAPABILITY_EVENTS=$((CAPABILITY_EVENTS+1))
  fi
  echo "$result"
}

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

get_status() { curl -sf "${BASE}/evolution/status" 2>/dev/null; }
get_fitness() { curl -sf "${BASE}/evolution/fitness?limit=10" 2>/dev/null; }

print_status() {
  local status=$(get_status)
  if [ -z "$status" ]; then
    echo -e "  ${RED}Failed to get status${NC}"
    return
  fi
  local gen=$(echo "$status" | jq_ "['current_generation']")
  local trend=$(echo "$status" | jq_ "['trend']")
  local fitness=$(echo "$status" | jq_ "['fitness']")
  echo -e "  ${BOLD}Gen${NC}=$gen  ${BOLD}Trend${NC}=$trend  ${BOLD}Fitness${NC}=$fitness"
}

# Add noise to a base value (simulates real-world variance)
noise() {
  local base="$1" range="${2:-0.05}"
  python3 -c "
import random
b = $base
r = $range
v = b + random.uniform(-r, r)
print(f'{max(0.0, min(1.0, v)):.4f}')
"
}

# ══════════════════════════════════════════════════════════════
# MAIN
# ══════════════════════════════════════════════════════════════

echo -e "\n${CYAN}╔══════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║   Exiv Evolution Engine — 30-Minute Benchmark Test   ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════════════╝${NC}\n"

# Pre-flight
if ! curl -sf "${BASE}/system/version" > /dev/null 2>&1; then
  echo -e "${RED}ERROR: Server not reachable at ${BASE}${NC}"
  exit 1
fi

version=$(curl -sf "${BASE}/system/version" | jq_ "['version']")
echo -e "${GREEN}Server running${NC} (v${version})"
echo -e "Start: $(date '+%Y-%m-%d %H:%M:%S')"
echo -e "Duration: 30 minutes ($DURATION seconds)"
echo -e "Interval: ${INTERVAL}s between evaluations (~180 evaluations total)\n"

# Initial status
echo -e "${CYAN}── Initial State ──${NC}"
print_status
echo ""

# ── Phase 1: Warm-up (0-5 min) ──
# Steady baseline: moderate scores, no drama
phase1_end=300
echo -e "${YELLOW}▶ Phase 1: Warm-up (0:00 - 5:00)${NC}"
echo -e "  Steady baseline interactions, moderate scores"

while [ $(elapsed) -lt $phase1_end ] && [ $(elapsed) -lt $DURATION ]; do
  cog=$(noise 0.50 0.05)
  beh=$(noise 0.55 0.05)
  saf=1.0
  aut=$(noise 0.35 0.03)
  meta=$(noise 0.45 0.05)

  result=$(evaluate "$cog" "$beh" "$saf" "$aut" "$meta" 2>/dev/null) || true

  # Progress dot
  printf "."

  # Status report every 60s
  if [ $((EVAL_COUNT % 6)) -eq 0 ] && [ $EVAL_COUNT -gt 0 ]; then
    echo ""
    echo -e "  [$(elapsed_fmt)] Evals=$EVAL_COUNT Gens=$GEN_EVENTS"
    print_status
  fi

  sleep "$INTERVAL"
done
echo ""
echo -e "  ${GREEN}Phase 1 complete${NC} — Evals=$EVAL_COUNT Gens=$GEN_EVENTS"
print_status
echo ""

# ── Phase 2: Growth (5-12 min) ──
# Gradual improvement: scores climb over time
phase2_end=720
echo -e "${YELLOW}▶ Phase 2: Growth (5:00 - 12:00)${NC}"
echo -e "  Gradual improvement, testing generation triggers"

p2_start_evals=$EVAL_COUNT
while [ $(elapsed) -lt $phase2_end ] && [ $(elapsed) -lt $DURATION ]; do
  # Progress factor: 0.0 → 1.0 over the phase
  progress=$(python3 -c "print(min(1.0, ($(elapsed) - $phase1_end) / ($phase2_end - $phase1_end)))")

  # Scores improve gradually
  cog=$(python3 -c "import random; print(f'{max(0.0, min(1.0, 0.50 + 0.30 * $progress + random.uniform(-0.03, 0.03))):.4f}')")
  beh=$(python3 -c "import random; print(f'{max(0.0, min(1.0, 0.55 + 0.30 * $progress + random.uniform(-0.03, 0.03))):.4f}')")
  saf=1.0
  aut=$(python3 -c "import random; print(f'{max(0.0, min(1.0, 0.35 + 0.35 * $progress + random.uniform(-0.02, 0.02))):.4f}')")
  meta=$(python3 -c "import random; print(f'{max(0.0, min(1.0, 0.45 + 0.25 * $progress + random.uniform(-0.03, 0.03))):.4f}')")

  result=$(evaluate "$cog" "$beh" "$saf" "$aut" "$meta" 2>/dev/null) || true
  printf "."

  if [ $((EVAL_COUNT % 6)) -eq 0 ]; then
    echo ""
    echo -e "  [$(elapsed_fmt)] Evals=$EVAL_COUNT Gens=$GEN_EVENTS (phase +$((EVAL_COUNT - p2_start_evals)))"
    print_status
  fi

  sleep "$INTERVAL"
done
echo ""
echo -e "  ${GREEN}Phase 2 complete${NC} — Gens=$GEN_EVENTS (+$((EVAL_COUNT - p2_start_evals)) evals)"
print_status
echo ""

# ── Phase 3: Stress (12-16 min) ──
# Safety breach → rollback → recovery
phase3_end=960
echo -e "${YELLOW}▶ Phase 3: Stress Test (12:00 - 16:00)${NC}"
echo -e "  Safety breach, rollback, agent recovery"

p3_start_evals=$EVAL_COUNT
p3_breach_done=false

while [ $(elapsed) -lt $phase3_end ] && [ $(elapsed) -lt $DURATION ]; do
  e=$(elapsed)

  if [ "$p3_breach_done" = false ] && [ $((e - 720)) -gt 30 ]; then
    # Trigger safety breach
    echo ""
    echo -e "  ${RED}⚡ Injecting safety breach...${NC}"
    result=$(evaluate 0.80 0.80 0.0 0.60 0.60 2>/dev/null) || true
    echo -e "  Breach result: $(echo "$result" | python3 -c "import sys,json; d=json.load(sys.stdin); print([e.get('type','?') for e in d.get('events',[])])" 2>/dev/null || echo "?")"
    p3_breach_done=true

    # Re-enable agent after breach
    sleep 2
    echo -e "  ${CYAN}🔄 Re-enabling agent...${NC}"
    reenable_agent
    sleep 1

    # Send recovery evaluations
    echo -e "  ${CYAN}🔄 Recovery sequence...${NC}"
    for i in $(seq 1 5); do
      evaluate 0.60 0.60 1.0 0.40 0.40 > /dev/null 2>&1 || true
      sleep 2
    done
  else
    # Normal recovery evaluations
    cog=$(noise 0.65 0.05)
    beh=$(noise 0.65 0.05)
    saf=1.0
    aut=$(noise 0.45 0.05)
    meta=$(noise 0.50 0.05)
    evaluate "$cog" "$beh" "$saf" "$aut" "$meta" > /dev/null 2>&1 || true
    printf "."
  fi

  if [ $((EVAL_COUNT % 6)) -eq 0 ]; then
    echo ""
    echo -e "  [$(elapsed_fmt)] Evals=$EVAL_COUNT Gens=$GEN_EVENTS Breaches=$BREACH_EVENTS Rollbacks=$ROLLBACK_EVENTS"
    print_status
  fi

  sleep "$INTERVAL"
done
echo ""
echo -e "  ${GREEN}Phase 3 complete${NC} — Breaches=$BREACH_EVENTS Rollbacks=$ROLLBACK_EVENTS"
print_status
echo ""

# ── Phase 4: Mastery (16-22 min) ──
# High performance + capability gain via new plugin snapshot
phase4_end=1320
echo -e "${YELLOW}▶ Phase 4: Mastery (16:00 - 22:00)${NC}"
echo -e "  High performance, capability gain test"

p4_start_evals=$EVAL_COUNT
p4_cap_done=false

while [ $(elapsed) -lt $phase4_end ] && [ $(elapsed) -lt $DURATION ]; do
  # High scores
  cog=$(noise 0.85 0.03)
  beh=$(noise 0.88 0.03)
  saf=1.0
  aut=$(noise 0.75 0.05)
  meta=$(noise 0.70 0.05)

  # Try capability gain halfway through phase
  if [ "$p4_cap_done" = false ] && [ $(elapsed) -gt 1150 ]; then
    echo ""
    echo -e "  ${MAGENTA}🧬 Injecting capability gain (new plugin)...${NC}"
    snapshot='{"active_plugins":["mind.deepseek","vision.new_capability","tool.code_analysis"],"plugin_capabilities":{"mind.deepseek":["Reasoning"],"vision.new_capability":["Vision"],"tool.code_analysis":["Tool"]},"personality_hash":"benchmark","strategy_params":{}}'
    result=$(evaluate "$cog" "$beh" "$saf" "$aut" "$meta" "$snapshot" 2>/dev/null) || true
    echo -e "  Result: $(echo "$result" | python3 -c "import sys,json; d=json.load(sys.stdin); print([e.get('type','?') for e in d.get('events',[])])" 2>/dev/null || echo "?")"
    p4_cap_done=true
  else
    evaluate "$cog" "$beh" "$saf" "$aut" "$meta" > /dev/null 2>&1 || true
    printf "."
  fi

  if [ $((EVAL_COUNT % 6)) -eq 0 ]; then
    echo ""
    echo -e "  [$(elapsed_fmt)] Evals=$EVAL_COUNT Gens=$GEN_EVENTS Caps=$CAPABILITY_EVENTS"
    print_status
  fi

  sleep "$INTERVAL"
done
echo ""
echo -e "  ${GREEN}Phase 4 complete${NC} — Capabilities=$CAPABILITY_EVENTS"
print_status
echo ""

# ── Phase 5: Volatility (22-27 min) ──
# Oscillating scores, test debounce and grace periods
phase5_end=1620
echo -e "${YELLOW}▶ Phase 5: Volatility (22:00 - 27:00)${NC}"
echo -e "  Score oscillations, debounce stress test"

p5_start_evals=$EVAL_COUNT

while [ $(elapsed) -lt $phase5_end ] && [ $(elapsed) -lt $DURATION ]; do
  # Oscillate between high and moderate scores
  cycle=$(python3 -c "import math; print(f'{(math.sin($(elapsed) / 30.0) + 1) / 2:.4f}')")

  cog=$(python3 -c "import random; print(f'{max(0.0, min(1.0, 0.55 + 0.30 * $cycle + random.uniform(-0.05, 0.05))):.4f}')")
  beh=$(python3 -c "import random; print(f'{max(0.0, min(1.0, 0.60 + 0.25 * $cycle + random.uniform(-0.05, 0.05))):.4f}')")
  saf=1.0
  aut=$(python3 -c "import random; print(f'{max(0.0, min(1.0, 0.40 + 0.30 * $cycle + random.uniform(-0.03, 0.03))):.4f}')")
  meta=$(python3 -c "import random; print(f'{max(0.0, min(1.0, 0.50 + 0.20 * $cycle + random.uniform(-0.05, 0.05))):.4f}')")

  evaluate "$cog" "$beh" "$saf" "$aut" "$meta" > /dev/null 2>&1 || true
  printf "."

  if [ $((EVAL_COUNT % 6)) -eq 0 ]; then
    echo ""
    echo -e "  [$(elapsed_fmt)] Evals=$EVAL_COUNT Gens=$GEN_EVENTS Warnings=$WARNING_EVENTS"
    print_status
  fi

  sleep "$INTERVAL"
done
echo ""
echo -e "  ${GREEN}Phase 5 complete${NC} — Warnings=$WARNING_EVENTS"
print_status
echo ""

# ── Phase 6: Final (27-30 min) ──
# Stable high performance to end on a good note
phase6_end=$DURATION
echo -e "${YELLOW}▶ Phase 6: Final (27:00 - 30:00)${NC}"
echo -e "  Stable high performance, final data collection"

while [ $(elapsed) -lt $phase6_end ]; do
  cog=$(noise 0.82 0.02)
  beh=$(noise 0.85 0.02)
  saf=1.0
  aut=$(noise 0.70 0.03)
  meta=$(noise 0.68 0.03)

  evaluate "$cog" "$beh" "$saf" "$aut" "$meta" > /dev/null 2>&1 || true
  printf "."

  if [ $((EVAL_COUNT % 6)) -eq 0 ]; then
    echo ""
    echo -e "  [$(elapsed_fmt)] Evals=$EVAL_COUNT"
    print_status
  fi

  sleep "$INTERVAL"
done
echo ""

# ══════════════════════════════════════════════════════════════
# FINAL REPORT
# ══════════════════════════════════════════════════════════════

echo -e "\n${CYAN}╔══════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║              30-Minute Benchmark Report               ║${NC}"
echo -e "${CYAN}╚══════════════════════════════════════════════════════╝${NC}\n"

echo -e "  ${BOLD}Duration${NC}:      $(elapsed_fmt) ($(elapsed)s)"
echo -e "  ${BOLD}Evaluations${NC}:  $EVAL_COUNT"
echo -e "  ${BOLD}Errors${NC}:       $ERRORS"
echo ""

# Final evolution status
status=$(get_status)
if [ -n "$status" ]; then
  gen=$(echo "$status" | jq_ "['current_generation']")
  trend=$(echo "$status" | jq_ "['trend']")
  fitness=$(echo "$status" | jq_ "['fitness']")
  echo -e "  ${BOLD}Final Generation${NC}:  $gen"
  echo -e "  ${BOLD}Final Trend${NC}:       $trend"
  echo -e "  ${BOLD}Final Fitness${NC}:     $fitness"
fi
echo ""

echo -e "  ${BOLD}Event Summary${NC}:"
echo -e "    Generations:    $GEN_EVENTS"
echo -e "    Warnings:       $WARNING_EVENTS"
echo -e "    Breaches:       $BREACH_EVENTS"
echo -e "    Rollbacks:      $ROLLBACK_EVENTS"
echo -e "    Capabilities:   $CAPABILITY_EVENTS"
echo ""

# Generation history
gens=$(curl -sf "${BASE}/evolution/generations?limit=100" 2>/dev/null)
if [ -n "$gens" ]; then
  gen_count=$(echo "$gens" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null)
  echo -e "  ${BOLD}Generation History${NC}: $gen_count records"
  echo "$gens" | python3 -c "
import sys, json
gens = json.load(sys.stdin)
for g in gens[-10:]:  # Show last 10
    print(f'    Gen {g[\"generation\"]:>3} | trigger={g[\"trigger\"]:<24} | fitness={g[\"fitness\"]:.4f}')
" 2>/dev/null || true
fi
echo ""

# Fitness timeline summary
fitness_data=$(curl -sf "${BASE}/evolution/fitness?limit=1000" 2>/dev/null)
if [ -n "$fitness_data" ]; then
  echo "$fitness_data" | python3 -c "
import sys, json
entries = json.load(sys.stdin)
if not entries:
    print('  No fitness data')
    sys.exit(0)

scores = [e['weighted_fitness'] for e in entries]
print(f'  Fitness Timeline: {len(entries)} entries')
print(f'    Min:     {min(scores):.4f}')
print(f'    Max:     {max(scores):.4f}')
print(f'    Avg:     {sum(scores)/len(scores):.4f}')
print(f'    First:   {scores[0]:.4f}')
print(f'    Last:    {scores[-1]:.4f}')

# Trend analysis
if len(scores) >= 10:
    first_10 = sum(scores[:10]) / 10
    last_10 = sum(scores[-10:]) / 10
    delta = last_10 - first_10
    direction = '📈 Improving' if delta > 0.01 else ('📉 Declining' if delta < -0.01 else '➡️  Stable')
    print(f'    Trend:   {direction} (Δ={delta:+.4f})')
" 2>/dev/null || true
fi
echo ""

# Rollback history
rollbacks=$(curl -sf "${BASE}/evolution/rollbacks" 2>/dev/null)
if [ -n "$rollbacks" ]; then
  rb_count=$(echo "$rollbacks" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null)
  echo -e "  ${BOLD}Rollbacks${NC}: $rb_count"
  if [ "$rb_count" -gt 0 ] 2>/dev/null; then
    echo "$rollbacks" | python3 -c "
import sys, json
rbs = json.load(sys.stdin)
for rb in rbs:
    print(f'    Gen {rb[\"from_generation\"]} → {rb[\"to_generation\"]} | {rb[\"reason\"]}')
" 2>/dev/null || true
  fi
fi
echo ""

# Dashboard check
echo -e "  ${BOLD}Dashboard${NC}: http://127.0.0.1:8081/evolution"
echo ""

# Pass/Fail
if [ $ERRORS -gt $((EVAL_COUNT / 10)) ]; then
  echo -e "  ${RED}BENCHMARK RESULT: HIGH ERROR RATE ($ERRORS errors / $EVAL_COUNT evals)${NC}"
  exit 1
elif [ "$GEN_EVENTS" -eq 0 ]; then
  echo -e "  ${RED}BENCHMARK RESULT: NO GENERATIONS CREATED${NC}"
  exit 1
else
  echo -e "  ${GREEN}BENCHMARK RESULT: PASSED${NC}"
  echo -e "  ${GREEN}$EVAL_COUNT evaluations, $GEN_EVENTS generations, ${ERRORS} errors in $(elapsed_fmt)${NC}"
fi
echo ""
