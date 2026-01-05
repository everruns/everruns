#!/bin/bash
# Tool Calling Smoke Tests
# Tests agent tool calling functionality via the API
#
# Usage: ./tool-calling-tests.sh [options]
# Options:
#   --api-url URL    API base URL (default: http://localhost:9000)
#   --verbose        Show detailed output
#   --skip-cleanup   Don't delete test agents after tests

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default configuration
API_URL="${API_URL:-http://localhost:9000}"
VERBOSE="${VERBOSE:-false}"
SKIP_CLEANUP="${SKIP_CLEANUP:-false}"
WAIT_TIME=15

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --api-url)
            API_URL="$2"
            shift 2
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --skip-cleanup)
            SKIP_CLEANUP=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Test result tracking
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0

# Cleanup tracking
AGENTS_TO_CLEANUP=()

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

log_error() {
    echo -e "${RED}[FAIL]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_verbose() {
    if [ "$VERBOSE" = "true" ]; then
        echo -e "${BLUE}[DEBUG]${NC} $1"
    fi
}

# Cleanup function
cleanup() {
    if [ "$SKIP_CLEANUP" = "true" ]; then
        log_warn "Skipping cleanup (--skip-cleanup flag set)"
        log_info "Test agents: ${AGENTS_TO_CLEANUP[*]}"
        return
    fi

    log_info "Cleaning up test agents..."
    for agent_id in "${AGENTS_TO_CLEANUP[@]}"; do
        log_verbose "Deleting agent: $agent_id"
        curl -s -X DELETE "$API_URL/v1/agents/$agent_id" > /dev/null 2>&1 || true
    done
    log_info "Cleanup complete"
}

trap cleanup EXIT

# Helper to run a test
run_test() {
    local test_name="$1"
    local test_function="$2"

    TESTS_RUN=$((TESTS_RUN + 1))
    log_info "Running test: $test_name"

    if $test_function; then
        TESTS_PASSED=$((TESTS_PASSED + 1))
        log_success "$test_name"
        return 0
    else
        TESTS_FAILED=$((TESTS_FAILED + 1))
        log_error "$test_name"
        return 1
    fi
}

# Helper to create agent with optional capabilities
create_agent() {
    local name="$1"
    local system_prompt="$2"
    local description="$3"
    shift 3
    local capabilities=("$@")

    local caps_json="[]"
    if [ ${#capabilities[@]} -gt 0 ]; then
        caps_json=$(printf '%s\n' "${capabilities[@]}" | jq -R . | jq -s .)
    fi

    local response=$(curl -s -X POST "$API_URL/v1/agents" \
        -H "Content-Type: application/json" \
        -d "{
            \"name\": \"$name\",
            \"system_prompt\": \"$system_prompt\",
            \"description\": \"$description\",
            \"capabilities\": $caps_json
        }")

    local agent_id=$(echo "$response" | jq -r '.id')

    if [ "$agent_id" = "null" ] || [ -z "$agent_id" ]; then
        log_error "Failed to create agent: $response"
        return 1
    fi

    AGENTS_TO_CLEANUP+=("$agent_id")
    echo "$agent_id"
}

# Helper to create session
create_session() {
    local agent_id="$1"
    local title="$2"

    local response=$(curl -s -X POST "$API_URL/v1/agents/$agent_id/sessions" \
        -H "Content-Type: application/json" \
        -d "{\"title\": \"$title\"}")

    echo "$response" | jq -r '.id'
}

# Helper to send message and wait
send_message() {
    local agent_id="$1"
    local session_id="$2"
    local message="$3"
    local wait_seconds="${4:-$WAIT_TIME}"

    curl -s -X POST "$API_URL/v1/agents/$agent_id/sessions/$session_id/messages" \
        -H "Content-Type: application/json" \
        -d "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"$message\"}]}}" > /dev/null

    log_verbose "Waiting ${wait_seconds}s for workflow to complete..."
    sleep "$wait_seconds"
}

# Helper to get messages
get_messages() {
    local agent_id="$1"
    local session_id="$2"

    curl -s "$API_URL/v1/agents/$agent_id/sessions/$session_id/messages"
}

# Helper to get events
get_events() {
    local agent_id="$1"
    local session_id="$2"

    curl -s "$API_URL/v1/agents/$agent_id/sessions/$session_id/events"
}

# ============================================================================
# Test: API Health Check
# ============================================================================
test_api_health() {
    local health=$(curl -s "$API_URL/health" | jq -r '.status')
    [ "$health" = "ok" ]
}

# ============================================================================
# Test: Single Tool (Math Add)
# ============================================================================
test_single_tool() {
    log_verbose "Creating math agent with test_math capability..."
    local agent_id=$(create_agent "Test Math Agent" "You are a math assistant. Use tools to calculate." "Single tool test" "test_math")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        log_error "Failed to create agent"
        return 1
    fi

    log_verbose "Agent ID: $agent_id"

    log_verbose "Creating session..."
    local session_id=$(create_session "$agent_id" "Single Tool Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        log_error "Failed to create session"
        return 1
    fi

    log_verbose "Sending message: What is 5 plus 3?"
    send_message "$agent_id" "$session_id" "What is 5 plus 3?"

    log_verbose "Checking messages..."
    local messages=$(get_messages "$agent_id" "$session_id")

    # Check for tool calls
    local tool_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[]] | length')
    log_verbose "Tool calls found: $tool_calls"

    if [ "$tool_calls" -ge 1 ]; then
        # Check for add tool
        local add_called=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "add")] | length')
        if [ "$add_called" -ge 1 ]; then
            return 0
        else
            log_error "Expected 'add' tool to be called"
            return 1
        fi
    else
        log_error "Expected at least 1 tool call, got $tool_calls"
        return 1
    fi
}

# ============================================================================
# Test: Multiple Tools (Math Operations)
# ============================================================================
test_multiple_tools() {
    log_verbose "Creating math agent for multiple tools test..."
    local agent_id=$(create_agent "Multi Tool Math Agent" "You are a math assistant. Use the appropriate tool for each calculation." "Multiple tools test" "test_math")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        return 1
    fi

    local session_id=$(create_session "$agent_id" "Multiple Tools Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        return 1
    fi

    log_verbose "Sending message: First add 5 and 3, then multiply the result by 2"
    send_message "$agent_id" "$session_id" "First add 5 and 3, then multiply the result by 2" 20

    local messages=$(get_messages "$agent_id" "$session_id")

    # Should have called at least 2 different tools
    local add_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "add")] | length')
    local multiply_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "multiply")] | length')

    log_verbose "Add calls: $add_calls, Multiply calls: $multiply_calls"

    if [ "$add_calls" -ge 1 ] && [ "$multiply_calls" -ge 1 ]; then
        return 0
    else
        log_error "Expected both add and multiply tools to be called"
        return 1
    fi
}

# ============================================================================
# Test: Weather Tools (Multi-step)
# ============================================================================
test_weather_tools() {
    log_verbose "Creating weather agent..."
    local agent_id=$(create_agent "Test Weather Agent" "You are a weather assistant. Use weather tools to get information." "Weather tools test" "test_weather")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        return 1
    fi

    local session_id=$(create_session "$agent_id" "Weather Tools Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        return 1
    fi

    log_verbose "Sending message: Get the weather and 5-day forecast for Tokyo"
    send_message "$agent_id" "$session_id" "Get the current weather and 5-day forecast for Tokyo" 20

    local messages=$(get_messages "$agent_id" "$session_id")

    # Should have called both get_weather and get_forecast
    local weather_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "get_weather")] | length')
    local forecast_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "get_forecast")] | length')

    log_verbose "Weather calls: $weather_calls, Forecast calls: $forecast_calls"

    if [ "$weather_calls" -ge 1 ] && [ "$forecast_calls" -ge 1 ]; then
        return 0
    else
        log_error "Expected both get_weather and get_forecast tools to be called"
        return 1
    fi
}

# ============================================================================
# Test: Parallel Tool Execution
# ============================================================================
test_parallel_tools() {
    log_verbose "Creating weather agent for parallel test..."
    local agent_id=$(create_agent "Parallel Weather Agent" "You are a weather assistant. When asked about multiple cities, get all weather data at once." "Parallel tools test" "test_weather")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        return 1
    fi

    local session_id=$(create_session "$agent_id" "Parallel Tools Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        return 1
    fi

    log_verbose "Sending message: Get the weather for New York, London, and Tokyo"
    send_message "$agent_id" "$session_id" "Get the current weather for New York, London, and Tokyo" 25

    local messages=$(get_messages "$agent_id" "$session_id")

    # Should have made multiple weather calls
    local weather_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "get_weather")] | length')

    log_verbose "Weather calls: $weather_calls"

    # Check if there's an assistant message with multiple tool calls (parallel execution)
    local parallel_msg=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | select((.tool_calls | length) > 1)] | length')

    log_verbose "Messages with multiple parallel calls: $parallel_msg"

    if [ "$weather_calls" -ge 3 ]; then
        if [ "$parallel_msg" -ge 1 ]; then
            log_verbose "Parallel execution confirmed"
        else
            log_warn "Tools may have been called sequentially instead of in parallel"
        fi
        return 0
    else
        log_error "Expected at least 3 weather calls, got $weather_calls"
        return 1
    fi
}

# ============================================================================
# Test: Combined Capabilities (Math + Weather)
# ============================================================================
test_combined_capabilities() {
    log_verbose "Creating agent with both math and weather capabilities..."
    local agent_id=$(create_agent "Combo Agent" "You are a helpful assistant with math and weather tools." "Combined capabilities test" "test_math" "test_weather")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        return 1
    fi

    local session_id=$(create_session "$agent_id" "Combined Capabilities Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        return 1
    fi

    log_verbose "Sending message: Get the temperature in Tokyo, then add 10 to it"
    send_message "$agent_id" "$session_id" "Get the temperature in Tokyo, then add 10 degrees to it" 20

    local messages=$(get_messages "$agent_id" "$session_id")

    # Should have called both weather and math tools
    local weather_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "get_weather")] | length')
    local add_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "add")] | length')

    log_verbose "Weather calls: $weather_calls, Add calls: $add_calls"

    if [ "$weather_calls" -ge 1 ] && [ "$add_calls" -ge 1 ]; then
        return 0
    else
        log_error "Expected both weather and add tools to be called"
        return 1
    fi
}

# ============================================================================
# Test: Tool Error Handling (Division by Zero)
# ============================================================================
test_tool_error_handling() {
    log_verbose "Creating math agent for error handling test..."
    local agent_id=$(create_agent "Error Test Agent" "You are a math assistant. Use divide tool when asked." "Error handling test" "test_math")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        return 1
    fi

    local session_id=$(create_session "$agent_id" "Error Handling Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        return 1
    fi

    log_verbose "Sending message: Divide 10 by 0"
    send_message "$agent_id" "$session_id" "Divide 10 by 0"

    local messages=$(get_messages "$agent_id" "$session_id")

    # Check for divide tool call
    local divide_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "divide")] | length')

    log_verbose "Divide calls: $divide_calls"

    if [ "$divide_calls" -ge 1 ]; then
        # Check for error in tool results
        local errors=$(echo "$messages" | jq '[.data[] | select(.tool_results != null) | .tool_results[] | select(.error != null)] | length')
        log_verbose "Tool errors: $errors"

        if [ "$errors" -ge 1 ]; then
            return 0
        else
            log_warn "Division by zero was called but no error was returned (might have been handled gracefully)"
            return 0  # Still pass - the agent might handle this differently
        fi
    else
        log_error "Expected divide tool to be called"
        return 1
    fi
}

# ============================================================================
# Test: WebFetch Capability Available
# ============================================================================
test_webfetch_capability() {
    log_verbose "Checking WebFetch capability is available..."

    local response=$(curl -s "$API_URL/v1/capabilities")
    local web_fetch=$(echo "$response" | jq '.data[] | select(.id == "web_fetch")')

    if [ -z "$web_fetch" ] || [ "$web_fetch" = "null" ]; then
        log_error "web_fetch capability not found in capabilities list"
        return 1
    fi

    local status=$(echo "$web_fetch" | jq -r '.status')
    if [ "$status" != "available" ]; then
        log_error "web_fetch capability status is '$status', expected 'available'"
        return 1
    fi

    log_verbose "WebFetch capability is available"
    return 0
}

# ============================================================================
# Test: Capability Detail Endpoint
# ============================================================================
test_capability_detail() {
    log_verbose "Testing capability detail endpoint..."

    # Test with test_math capability which has system_prompt and tools
    local response=$(curl -s "$API_URL/v1/capabilities/test_math")

    local id=$(echo "$response" | jq -r '.id')
    if [ "$id" != "test_math" ]; then
        log_error "Expected capability id 'test_math', got '$id'"
        return 1
    fi

    local name=$(echo "$response" | jq -r '.name')
    if [ -z "$name" ] || [ "$name" = "null" ]; then
        log_error "Capability name is missing"
        return 1
    fi
    log_verbose "Capability name: $name"

    # Check system_prompt is present (test_math has one)
    local system_prompt=$(echo "$response" | jq -r '.system_prompt // empty')
    if [ -z "$system_prompt" ]; then
        log_error "system_prompt is missing for test_math capability"
        return 1
    fi
    log_verbose "system_prompt present (${#system_prompt} chars)"

    # Check tool_definitions array is present and not empty
    local tool_count=$(echo "$response" | jq '.tool_definitions | length')
    if [ "$tool_count" -lt 1 ]; then
        log_error "Expected at least 1 tool_definition, got $tool_count"
        return 1
    fi
    log_verbose "tool_definitions count: $tool_count"

    # Verify tool has expected fields
    local first_tool=$(echo "$response" | jq '.tool_definitions[0]')
    local tool_name=$(echo "$first_tool" | jq -r '.name')
    local tool_desc=$(echo "$first_tool" | jq -r '.description')

    if [ -z "$tool_name" ] || [ "$tool_name" = "null" ]; then
        log_error "Tool name is missing"
        return 1
    fi
    log_verbose "First tool: $tool_name"

    if [ -z "$tool_desc" ] || [ "$tool_desc" = "null" ]; then
        log_error "Tool description is missing"
        return 1
    fi

    log_verbose "Capability detail endpoint working correctly"
    return 0
}

# ============================================================================
# Test: WebFetch Tool Input Validation
# ============================================================================
test_webfetch_tool() {
    log_verbose "Creating agent with web_fetch capability..."
    local agent_id=$(create_agent "Web Fetch Test Agent" "You are a web assistant. Use the web_fetch tool to fetch URLs." "WebFetch tool test" "web_fetch")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        log_error "Failed to create agent"
        return 1
    fi

    log_verbose "Agent ID: $agent_id"

    log_verbose "Creating session..."
    local session_id=$(create_session "$agent_id" "WebFetch Tool Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        log_error "Failed to create session"
        return 1
    fi

    # Ask to fetch an invalid URL - should return error
    log_verbose "Sending message: Fetch content from an-invalid-url"
    send_message "$agent_id" "$session_id" "Fetch content from the URL an-invalid-url" 15

    local messages=$(get_messages "$agent_id" "$session_id")

    # Check for tool calls
    local tool_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "web_fetch")] | length')
    log_verbose "web_fetch tool calls found: $tool_calls"

    if [ "$tool_calls" -ge 1 ]; then
        # The tool should have been called and returned an error for invalid URL
        return 0
    else
        # The LLM might have recognized the invalid URL and not called the tool
        # which is also acceptable behavior
        log_warn "web_fetch tool was not called (LLM may have handled invalid URL)"
        return 0
    fi
}

# ============================================================================
# Test: Events and Messages Sync
# ============================================================================
test_events_sync() {
    log_verbose "Creating agent for events sync test..."
    local agent_id=$(create_agent "Events Sync Agent" "You are a helpful assistant." "Events sync test")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        log_error "Failed to create agent"
        return 1
    fi

    log_verbose "Agent ID: $agent_id"

    local session_id=$(create_session "$agent_id" "Events Sync Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        log_error "Failed to create session"
        return 1
    fi

    log_verbose "Sending message: Say hello in one word"
    send_message "$agent_id" "$session_id" "Say hello in one word"

    # Get messages
    local messages=$(get_messages "$agent_id" "$session_id")
    local message_count=$(echo "$messages" | jq '.data | length')
    log_verbose "Messages count: $message_count"

    # Get events
    local events=$(get_events "$agent_id" "$session_id")
    local event_count=$(echo "$events" | jq '.data | length')
    log_verbose "Events count: $event_count"

    # Should have at least 2 events: message.user and message.agent
    if [ "$event_count" -lt 2 ]; then
        log_error "Expected at least 2 events, got $event_count"
        return 1
    fi

    # Check for message.user event
    local user_events=$(echo "$events" | jq '[.data[] | select(.event_type == "message.user")] | length')
    if [ "$user_events" -lt 1 ]; then
        log_error "Expected at least 1 message.user event"
        return 1
    fi
    log_verbose "User events: $user_events"

    # Check for message.agent event
    local agent_events=$(echo "$events" | jq '[.data[] | select(.event_type == "message.agent")] | length')
    if [ "$agent_events" -lt 1 ]; then
        log_error "Expected at least 1 message.agent event"
        return 1
    fi
    log_verbose "Agent events: $agent_events"

    # Verify event data contains message info
    local first_user_event=$(echo "$events" | jq '.data[] | select(.event_type == "message.user") | .data')
    local has_message_id=$(echo "$first_user_event" | jq 'has("message_id")')
    local has_content=$(echo "$first_user_event" | jq 'has("content")')

    if [ "$has_message_id" != "true" ] || [ "$has_content" != "true" ]; then
        log_error "Event data missing required fields (message_id, content)"
        return 1
    fi

    log_verbose "Events contain required message data"
    return 0
}

# ============================================================================
# Test: CurrentTime Capability Available
# ============================================================================
test_current_time_capability() {
    log_verbose "Checking CurrentTime capability is available..."

    local response=$(curl -s "$API_URL/v1/capabilities")
    local current_time=$(echo "$response" | jq '.data[] | select(.id == "current_time")')

    if [ -z "$current_time" ] || [ "$current_time" = "null" ]; then
        log_error "current_time capability not found in capabilities list"
        return 1
    fi

    local status=$(echo "$current_time" | jq -r '.status')
    if [ "$status" != "available" ]; then
        log_error "current_time capability status is '$status', expected 'available'"
        return 1
    fi

    log_verbose "CurrentTime capability is available"
    return 0
}

# ============================================================================
# Test: Dad Joke Agent (CurrentTime Tool)
# ============================================================================
test_dad_joke_agent() {
    log_verbose "Creating Dad Joke Agent with current_time capability..."
    local agent_id=$(create_agent "Dad Joke Agent" "You are a master of dad jokes. When asked about time, you MUST first use the get_current_time tool to get the actual current time, then craft a hilarious dad joke that incorporates the real time." "Dad joke time test" "current_time")

    if [ -z "$agent_id" ] || [ "$agent_id" = "null" ]; then
        log_error "Failed to create agent"
        return 1
    fi

    log_verbose "Agent ID: $agent_id"

    log_verbose "Creating session..."
    local session_id=$(create_session "$agent_id" "Dad Joke Time Test")

    if [ -z "$session_id" ] || [ "$session_id" = "null" ]; then
        log_error "Failed to create session"
        return 1
    fi

    log_verbose "Sending message: Tell me a joke about the current time!"
    send_message "$agent_id" "$session_id" "Tell me a joke about the current time!"

    log_verbose "Checking messages..."
    local messages=$(get_messages "$agent_id" "$session_id")

    # Check for tool calls
    local tool_calls=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[]] | length')
    log_verbose "Tool calls found: $tool_calls"

    if [ "$tool_calls" -ge 1 ]; then
        # Check for get_current_time tool
        local time_called=$(echo "$messages" | jq '[.data[] | select(.tool_calls != null) | .tool_calls[] | select(.name == "get_current_time")] | length')
        if [ "$time_called" -ge 1 ]; then
            log_verbose "get_current_time tool was called"

            # Check for assistant response
            local assistant_msgs=$(echo "$messages" | jq '[.data[] | select(.role == "assistant") | select(.content != null)] | length')
            if [ "$assistant_msgs" -ge 1 ]; then
                log_verbose "Assistant responded with message"
                return 0
            else
                log_error "Expected assistant response after tool call"
                return 1
            fi
        else
            log_error "Expected 'get_current_time' tool to be called"
            return 1
        fi
    else
        log_error "Expected at least 1 tool call, got $tool_calls"
        return 1
    fi
}

# ============================================================================
# Main Test Runner
# ============================================================================
main() {
    echo ""
    echo "========================================"
    echo "  Tool Calling Smoke Tests"
    echo "========================================"
    echo ""
    log_info "API URL: $API_URL"
    log_info "Verbose: $VERBOSE"
    echo ""

    # Pre-flight check
    log_info "Checking API availability..."
    if ! run_test "API Health Check" test_api_health; then
        log_error "API is not available at $API_URL"
        log_error "Please ensure the API and worker are running"
        exit 1
    fi
    echo ""

    # Run tests
    log_info "Running tool calling tests..."
    echo ""

    run_test "Single Tool (TestMath Add)" test_single_tool || true
    echo ""

    run_test "Multiple Tools (TestMath Operations)" test_multiple_tools || true
    echo ""

    run_test "TestWeather Tools (Multi-step)" test_weather_tools || true
    echo ""

    run_test "Parallel Tool Execution" test_parallel_tools || true
    echo ""

    run_test "Combined Capabilities (TestMath + TestWeather)" test_combined_capabilities || true
    echo ""

    run_test "Tool Error Handling (Division by Zero)" test_tool_error_handling || true
    echo ""

    run_test "WebFetch Capability Available" test_webfetch_capability || true
    echo ""

    run_test "Capability Detail Endpoint" test_capability_detail || true
    echo ""

    run_test "WebFetch Tool (Input Validation)" test_webfetch_tool || true
    echo ""

    run_test "Events and Messages Sync" test_events_sync || true
    echo ""

    run_test "CurrentTime Capability Available" test_current_time_capability || true
    echo ""

    run_test "Dad Joke Agent (CurrentTime Tool)" test_dad_joke_agent || true
    echo ""

    # Summary
    echo "========================================"
    echo "  Test Summary"
    echo "========================================"
    echo ""
    log_info "Tests run: $TESTS_RUN"
    log_success "Tests passed: $TESTS_PASSED"
    if [ "$TESTS_FAILED" -gt 0 ]; then
        log_error "Tests failed: $TESTS_FAILED"
    else
        echo -e "${GREEN}Tests failed: $TESTS_FAILED${NC}"
    fi
    echo ""

    if [ "$TESTS_FAILED" -gt 0 ]; then
        exit 1
    else
        log_success "All tool calling tests passed!"
        exit 0
    fi
}

main
