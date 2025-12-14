#!/usr/bin/env bash
set -euo pipefail

# Quick API and UI testing script
# Usage: ./scripts/test-api.sh [--with-ui]
#
# Options:
#   --with-ui    Also test the UI at localhost:3000

API_URL="http://localhost:9000"
UI_URL="http://localhost:3000"
TEST_UI=false

# Parse arguments
for arg in "$@"; do
  case $arg in
    --with-ui)
      TEST_UI=true
      shift
      ;;
  esac
done

echo "🧪 Testing Everruns API"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "API URL: $API_URL"
echo ""

# Test 1: Health check
echo "1️⃣  Testing health endpoint..."
HEALTH=$(curl -s "$API_URL/health")
echo "   Status: $(echo $HEALTH | jq -r '.status')"
echo "   Version: $(echo $HEALTH | jq -r '.version')"
echo ""

# Test 2: Create agent
echo "2️⃣  Creating agent..."
AGENT=$(curl -s -X POST "$API_URL/v1/agents" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"Test Agent\",
    \"description\": \"Created by test script\",
    \"default_model_id\": \"gpt-5.1\",
    \"definition\": {
      \"system_prompt\": \"You are a helpful assistant\",
      \"temperature\": 0.7
    }
  }")

AGENT_ID=$(echo $AGENT | jq -r '.id')
echo "   Agent ID: $AGENT_ID"
echo "   Name: $(echo $AGENT | jq -r '.name')"
echo "   Status: $(echo $AGENT | jq -r '.status')"
echo ""

# Test 3: Get agent
echo "3️⃣  Retrieving agent..."
FETCHED=$(curl -s "$API_URL/v1/agents/$AGENT_ID")
echo "   Retrieved: $(echo $FETCHED | jq -r '.name')"
echo ""

# Test 4: Update agent
echo "4️⃣  Updating agent..."
UPDATED=$(curl -s -X PATCH "$API_URL/v1/agents/$AGENT_ID" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"Updated Test Agent\",
    \"description\": \"Updated by test script\"
  }")
echo "   New name: $(echo $UPDATED | jq -r '.name')"
echo ""

# Test 5: List agents
echo "5️⃣  Listing agents..."
AGENTS=$(curl -s "$API_URL/v1/agents")
COUNT=$(echo $AGENTS | jq '. | length')
echo "   Found $COUNT agent(s)"
echo ""

# Test 6: Create thread
echo "6️⃣  Creating thread..."
THREAD=$(curl -s -X POST "$API_URL/v1/threads" \
  -H "Content-Type: application/json" \
  -d "{}")
THREAD_ID=$(echo $THREAD | jq -r '.id')
echo "   Thread ID: $THREAD_ID"
echo ""

# Test 7: Add message
echo "7️⃣  Adding message to thread..."
MESSAGE=$(curl -s -X POST "$API_URL/v1/threads/$THREAD_ID/messages" \
  -H "Content-Type: application/json" \
  -d "{
    \"role\": \"user\",
    \"content\": \"Hello, world!\"
  }")
MESSAGE_ID=$(echo $MESSAGE | jq -r '.id')
echo "   Message ID: $MESSAGE_ID"
echo "   Content: $(echo $MESSAGE | jq -r '.content')"
echo ""

# Test 8: Create run
echo "8️⃣  Creating run..."
RUN=$(curl -s -X POST "$API_URL/v1/runs" \
  -H "Content-Type: application/json" \
  -d "{
    \"agent_id\": \"$AGENT_ID\",
    \"thread_id\": \"$THREAD_ID\"
  }")
RUN_ID=$(echo $RUN | jq -r '.id')
echo "   Run ID: $RUN_ID"
echo "   Status: $(echo $RUN | jq -r '.status')"
echo ""

# Wait for workflow to complete
echo "⏳ Waiting for workflow to complete..."
sleep 2

# Check run status
echo "🔍 Checking final run status..."
RUN_STATUS=$(curl -s "$API_URL/v1/runs/$RUN_ID")
echo "   Status: $(echo $RUN_STATUS | jq -r '.status')"
echo ""

# Test 10: OpenAPI spec
echo "🔟 Testing OpenAPI spec..."
SPEC=$(curl -s "$API_URL/api-doc/openapi.json")
TITLE=$(echo $SPEC | jq -r '.info.title')
echo "   API Title: $TITLE"
echo "   Endpoints: $(echo $SPEC | jq '.paths | keys | length')"
echo ""

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ API tests passed!"
echo ""

# UI Tests (optional)
if [ "$TEST_UI" = true ]; then
  echo ""
  echo "🖥️  Testing UI"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "UI URL: $UI_URL"
  echo ""

  # Test UI health - check if page loads
  echo "1️⃣  Testing UI availability..."
  UI_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" "$UI_URL" || echo "000")
  if [ "$UI_RESPONSE" = "200" ] || [ "$UI_RESPONSE" = "307" ]; then
    echo "   ✅ UI is responding (HTTP $UI_RESPONSE)"
  else
    echo "   ❌ UI not responding (HTTP $UI_RESPONSE)"
    echo "   Make sure UI is running: cd apps/ui && npm run dev"
    exit 1
  fi
  echo ""

  # Test dashboard page
  echo "2️⃣  Testing dashboard page..."
  DASH_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" "$UI_URL/dashboard")
  if [ "$DASH_RESPONSE" = "200" ]; then
    echo "   ✅ Dashboard page loads (HTTP $DASH_RESPONSE)"
  else
    echo "   ❌ Dashboard failed (HTTP $DASH_RESPONSE)"
    exit 1
  fi
  echo ""

  # Test agents page
  echo "3️⃣  Testing agents page..."
  AGENTS_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" "$UI_URL/agents")
  if [ "$AGENTS_RESPONSE" = "200" ]; then
    echo "   ✅ Agents page loads (HTTP $AGENTS_RESPONSE)"
  else
    echo "   ❌ Agents page failed (HTTP $AGENTS_RESPONSE)"
    exit 1
  fi
  echo ""

  # Test runs page
  echo "4️⃣  Testing runs page..."
  RUNS_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" "$UI_URL/runs")
  if [ "$RUNS_RESPONSE" = "200" ]; then
    echo "   ✅ Runs page loads (HTTP $RUNS_RESPONSE)"
  else
    echo "   ❌ Runs page failed (HTTP $RUNS_RESPONSE)"
    exit 1
  fi
  echo ""

  # Test chat page
  echo "5️⃣  Testing chat page..."
  CHAT_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" "$UI_URL/chat")
  if [ "$CHAT_RESPONSE" = "200" ]; then
    echo "   ✅ Chat page loads (HTTP $CHAT_RESPONSE)"
  else
    echo "   ❌ Chat page failed (HTTP $CHAT_RESPONSE)"
    exit 1
  fi
  echo ""

  # Test agent detail page (using the agent we created)
  echo "6️⃣  Testing agent detail page..."
  AGENT_PAGE_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" "$UI_URL/agents/$AGENT_ID")
  if [ "$AGENT_PAGE_RESPONSE" = "200" ]; then
    echo "   ✅ Agent detail page loads (HTTP $AGENT_PAGE_RESPONSE)"
  else
    echo "   ❌ Agent detail page failed (HTTP $AGENT_PAGE_RESPONSE)"
    exit 1
  fi
  echo ""

  # Test run detail page (using the run we created)
  echo "7️⃣  Testing run detail page..."
  RUN_PAGE_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" "$UI_URL/runs/$RUN_ID")
  if [ "$RUN_PAGE_RESPONSE" = "200" ]; then
    echo "   ✅ Run detail page loads (HTTP $RUN_PAGE_RESPONSE)"
  else
    echo "   ❌ Run detail page failed (HTTP $RUN_PAGE_RESPONSE)"
    exit 1
  fi
  echo ""

  # Test thread detail page
  echo "8️⃣  Testing thread detail page..."
  THREAD_PAGE_RESPONSE=$(curl -s -o /dev/null -w "%{http_code}" "$UI_URL/threads/$THREAD_ID")
  if [ "$THREAD_PAGE_RESPONSE" = "200" ]; then
    echo "   ✅ Thread detail page loads (HTTP $THREAD_PAGE_RESPONSE)"
  else
    echo "   ❌ Thread detail page failed (HTTP $THREAD_PAGE_RESPONSE)"
    exit 1
  fi
  echo ""

  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "✅ UI tests passed!"
  echo ""
fi

echo "📊 Summary:"
echo "   Agent: $AGENT_ID"
echo "   Thread: $THREAD_ID"
echo "   Message: $MESSAGE_ID"
echo "   Run: $RUN_ID ($(echo $RUN_STATUS | jq -r '.status'))"
echo ""
echo "💡 View API docs: $API_URL/swagger-ui/"
if [ "$TEST_UI" = true ]; then
  echo "💡 View UI: $UI_URL"
fi
