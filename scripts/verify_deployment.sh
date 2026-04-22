#!/bin/bash
# Deployment Verification Script
# Run this after deployment to verify everything is working

set -e

echo "=========================================="
echo "  Deployment Verification"
echo "=========================================="

# Configuration
if nc -z -w 1 192.168.2.200 22 2>/dev/null; then
    SERVER="root@192.168.2.200"
    SSH_PORT="22"
else
    SERVER="root@hackerlife.fun"
    SSH_PORT="222"
fi
NEXUS_URL="https://news.hackerlife.fun:8443"
APP_NAME="com.freshloop.cortex"
LAUNCH_DOMAIN="gui/$(id -u)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

pass() { echo -e "${GREEN}✓${NC} $1"; }
fail() { echo -e "${RED}✗${NC} $1"; }
warn() { echo -e "${YELLOW}!${NC} $1"; }

# 1. Check Nexus Remote Service
echo ""
echo ">>> Checking Nexus on NAS..."
if ssh -p $SSH_PORT $SERVER "systemctl is-active nexus" &>/dev/null; then
    pass "Nexus service is running"
else
    fail "Nexus service is NOT running"
    echo "  Try: ssh -p $SSH_PORT $SERVER 'systemctl status nexus'"
fi

# 2. Check Nexus Health Endpoint
echo ""
echo ">>> Checking Nexus API health..."
HEALTH_RESPONSE=$(curl -sk "$NEXUS_URL/api/health" 2>/dev/null || echo "FAILED")
if [[ "$HEALTH_RESPONSE" == *"ok"* ]] || [[ "$HEALTH_RESPONSE" == *"healthy"* ]]; then
    pass "Nexus API is healthy"
else
    fail "Nexus API health check failed: $HEALTH_RESPONSE"
fi

# 3. Check Cortex Local Service
echo ""
echo ">>> Checking Cortex locally..."
JOB_INFO=$(launchctl print "$LAUNCH_DOMAIN/$APP_NAME" 2>/dev/null || true)
TRIGGER_RESPONSE=$(curl -s "http://localhost:3721/api/status" 2>/dev/null || echo "FAILED")
if [ -n "$JOB_INFO" ]; then
    pass "Cortex service is loaded"
    JOB_PROGRAM=$(echo "$JOB_INFO" | awk -F' = ' '/^[[:space:]]*program = / { print $2; exit }')
    JOB_STATE=$(echo "$JOB_INFO" | awk -F' = ' '/^[[:space:]]*state = / { print $2; exit }')
    JOB_PID=$(echo "$JOB_INFO" | awk -F' = ' '/^[[:space:]]*pid = / { print $2; exit }')
    JOB_HOME=$(echo "$JOB_INFO" | awk -F'=> ' '/^[[:space:]]*HOME => / { print $2; exit }')
    [ -n "$JOB_PROGRAM" ] && echo "  Program: $JOB_PROGRAM"
    [ -n "$JOB_STATE" ] && echo "  State: $JOB_STATE"
else
    if [[ "$TRIGGER_RESPONSE" != "FAILED" ]] && [[ "$TRIGGER_RESPONSE" != "" ]]; then
        warn "Cortex API is responding, but launchctl job is not visible in this session"
    else
        fail "Cortex service is NOT loaded"
        echo "  Try: ./scripts/install_local_service.sh"
    fi
fi

# 4. Check Cortex Process
if [ -n "$JOB_PID" ] && [ "$JOB_PID" != "-" ]; then
    pass "Cortex process is running (PID: $JOB_PID)"
else
    if [[ "$TRIGGER_RESPONSE" != "FAILED" ]] && [[ "$TRIGGER_RESPONSE" != "" ]]; then
        warn "Cortex API is responding, but PID is not visible yet"
    else
        fail "Cortex process is NOT running"
    fi
fi

# 5. Check Cortex Trigger API
echo ""
echo ">>> Checking Cortex Trigger API..."
if [[ "$TRIGGER_RESPONSE" != "FAILED" ]] && [[ "$TRIGGER_RESPONSE" != "" ]]; then
    pass "Cortex Trigger API is responding"
    echo "  Status: $(echo $TRIGGER_RESPONSE | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('status', 'unknown'))" 2>/dev/null || echo 'parse error')"
else
    fail "Cortex Trigger API is not responding"
    echo "  The service may still be starting up"
fi

# 6. Check Configuration
echo ""
echo ">>> Checking configuration..."
if [ -f "config.toml" ]; then
    NEXUS_KEY=$(grep -A2 "^\[nexus\]" config.toml | grep "auth_key" | cut -d'"' -f2)
    NEXUS_API=$(grep -A2 "^\[nexus\]" config.toml | grep "api_url" | cut -d'"' -f2)
    pass "config.toml found"
    echo "  Nexus URL: $NEXUS_API"
    echo "  Auth Key: ${NEXUS_KEY:0:8}****"
else
    fail "config.toml not found"
fi

# Summary
echo ""
echo "=========================================="
echo "  Verification Complete"
echo "=========================================="
echo ""
echo "If all checks passed, your deployment is healthy!"
echo ""
echo "Quick Links:"
echo "  - News Site: https://news.hackerlife.fun"
echo "  - Admin Panel: https://news.hackerlife.fun/admin"
echo "  - Cortex Status: http://localhost:3721/api/status"
echo ""
echo "Troubleshooting:"
echo "  - Nexus logs: ssh -p $SSH_PORT $SERVER 'journalctl -u nexus -f'"
if [ -n "$JOB_HOME" ]; then
    echo "  - Cortex logs: tail -f $JOB_HOME/.freshloop/logs/cortex-$(date +%Y-%m-%d).log"
else
    echo "  - Cortex logs: tail -f ~/.freshloop/logs/cortex-$(date +%Y-%m-%d).log"
fi
