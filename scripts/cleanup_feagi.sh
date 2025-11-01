#!/bin/bash
# Copyright 2025 Neuraville Inc.
# Licensed under the Apache License, Version 2.0
#
# FEAGI Cleanup Script (Mac/Linux)
# 
# This script safely terminates all running FEAGI instances and releases ports.
# Usage: ./cleanup_feagi.sh [--force] [--yes]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# FEAGI ports to check/cleanup
PORTS=(
    8000    # HTTP API
    5555    # ZMQ REQ/REP
    5556    # ZMQ PUB/SUB
    5557    # ZMQ PUSH/PULL
    5558    # Sensory input
    5562    # Visualization
    5563    # ZMQ REST
    5564    # Motor output
)

echo -e "${BLUE}╔═══════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                    FEAGI Cleanup Utility                         ║${NC}"
echo -e "${BLUE}╚═══════════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Parse command line arguments
FORCE_KILL=false
AUTO_YES=false

for arg in "$@"; do
    case $arg in
        --force|-f)
            FORCE_KILL=true
            ;;
        --yes|-y)
            AUTO_YES=true
            ;;
        *)
            echo -e "${RED}Unknown argument: $arg${NC}"
            echo "Usage: $0 [--force] [--yes]"
            exit 1
            ;;
    esac
done

if [[ "$FORCE_KILL" == true ]]; then
    echo -e "${YELLOW}⚠️  Force mode enabled - will use SIGKILL immediately${NC}"
    echo ""
fi

# Function to check if a port is in use
check_port() {
    local port=$1
    lsof -i :"$port" -sTCP:LISTEN -t 2>/dev/null
}

# Function to get process info
get_process_info() {
    local pid=$1
    ps -p "$pid" -o comm=,args= 2>/dev/null || echo "Unknown process"
}

# Function to ask for permission
ask_permission() {
    local message=$1
    
    if [[ "$AUTO_YES" == true ]]; then
        return 0
    fi
    
    echo -e "${YELLOW}$message${NC}"
    read -p "Continue? [y/N] " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        return 1
    fi
    return 0
}

# Function to gracefully kill a process
kill_process() {
    local pid=$1
    local name=$2
    
    if ! ps -p "$pid" > /dev/null 2>&1; then
        echo -e "  ${YELLOW}Process $pid already terminated${NC}"
        return 0
    fi
    
    if [[ "$FORCE_KILL" == true ]]; then
        echo -e "  ${RED}Sending SIGKILL to $pid${NC}"
        kill -9 "$pid" 2>/dev/null || true
        sleep 0.5
    else
        echo -e "  ${YELLOW}Sending SIGTERM to $pid${NC}"
        kill -TERM "$pid" 2>/dev/null || true
        
        # Wait up to 5 seconds for graceful shutdown
        for i in {1..10}; do
            if ! ps -p "$pid" > /dev/null 2>&1; then
                echo -e "  ${GREEN}✓ Process $pid terminated gracefully${NC}"
                return 0
            fi
            sleep 0.5
        done
        
        # If still running, send SIGKILL
        if ps -p "$pid" > /dev/null 2>&1; then
            echo -e "  ${RED}Process $pid did not respond to SIGTERM, sending SIGKILL${NC}"
            kill -9 "$pid" 2>/dev/null || true
            sleep 0.5
        fi
    fi
    
    # Final check
    if ps -p "$pid" > /dev/null 2>&1; then
        echo -e "  ${RED}✗ Failed to terminate process $pid${NC}"
        return 1
    else
        echo -e "  ${GREEN}✓ Process $pid terminated${NC}"
        return 0
    fi
}

# Step 1: Find and kill FEAGI processes by name
echo -e "${BLUE}[1/3] Checking for FEAGI processes...${NC}"
FEAGI_PIDS=$(pgrep -f "feagi" | grep -v $$ | grep -v "cleanup_feagi" || true)

if [[ -z "$FEAGI_PIDS" ]]; then
    echo -e "${GREEN}  ✓ No FEAGI processes found${NC}"
    echo ""
else
    echo -e "${YELLOW}  Found FEAGI processes:${NC}"
    for pid in $FEAGI_PIDS; do
        proc_info=$(get_process_info "$pid")
        echo -e "    ${YELLOW}PID $pid:${NC} $proc_info"
    done
    echo ""
    
    if ! ask_permission "  Kill these FEAGI processes?"; then
        echo -e "${YELLOW}Skipping FEAGI process cleanup${NC}"
        echo ""
    else
        for pid in $FEAGI_PIDS; do
            kill_process "$pid" "FEAGI"
        done
        echo ""
    fi
fi

# Step 2: Check for Python processes running FEAGI
echo -e "${BLUE}[2/3] Checking for Python FEAGI processes...${NC}"
PYTHON_FEAGI_PIDS=$(pgrep -f "python.*feagi" | grep -v $$ || true)

if [[ -z "$PYTHON_FEAGI_PIDS" ]]; then
    echo -e "${GREEN}  ✓ No Python FEAGI processes found${NC}"
    echo ""
else
    echo -e "${YELLOW}  Found Python FEAGI processes:${NC}"
    for pid in $PYTHON_FEAGI_PIDS; do
        proc_info=$(get_process_info "$pid")
        echo -e "    ${YELLOW}PID $pid:${NC} $proc_info"
    done
    echo ""
    
    if ! ask_permission "  Kill these Python FEAGI processes?"; then
        echo -e "${YELLOW}Skipping Python FEAGI cleanup${NC}"
        echo ""
    else
        for pid in $PYTHON_FEAGI_PIDS; do
            kill_process "$pid" "Python FEAGI"
        done
        echo ""
    fi
fi

# Step 3: Check and clean up specific ports
echo -e "${BLUE}[3/3] Checking FEAGI ports...${NC}"
PORTS_IN_USE=false
PORT_PIDS_TO_KILL=()

for port in "${PORTS[@]}"; do
    port_pids=$(check_port "$port")
    
    if [[ -n "$port_pids" ]]; then
        PORTS_IN_USE=true
        echo -e "${YELLOW}  Port $port is in use:${NC}"
        
        for pid in $port_pids; do
            proc_info=$(get_process_info "$pid")
            echo -e "    ${YELLOW}PID $pid:${NC} $proc_info"
            
            # Only add FEAGI-related processes for cleanup
            if echo "$proc_info" | grep -iqE "feagi|cargo"; then
                PORT_PIDS_TO_KILL+=("$pid")
            else
                echo -e "    ${BLUE}(Non-FEAGI process, will skip)${NC}"
            fi
        done
    fi
done

if [[ "$PORTS_IN_USE" == false ]]; then
    echo -e "${GREEN}  ✓ All FEAGI ports are free${NC}"
    echo ""
else
    echo ""
    if [[ ${#PORT_PIDS_TO_KILL[@]} -gt 0 ]]; then
        if ! ask_permission "  Kill processes occupying FEAGI ports?"; then
            echo -e "${YELLOW}Skipping port cleanup${NC}"
            echo ""
        else
            for pid in "${PORT_PIDS_TO_KILL[@]}"; do
                kill_process "$pid" "Port occupant"
            done
            echo ""
        fi
    fi
fi

# Final verification
echo -e "${BLUE}═══════════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}Cleanup Summary${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════════${NC}"

ALL_CLEAR=true

# Check for any remaining FEAGI processes
REMAINING_FEAGI=$(pgrep -f "feagi" | grep -v $$ | grep -v "cleanup_feagi" || true)
if [[ -n "$REMAINING_FEAGI" ]]; then
    echo -e "${YELLOW}⚠️  FEAGI processes still running (skipped or failed to terminate):${NC}"
    for pid in $REMAINING_FEAGI; do
        proc_info=$(get_process_info "$pid")
        echo -e "  PID $pid: $proc_info"
    done
    ALL_CLEAR=false
else
    echo -e "${GREEN}✓ No FEAGI processes running${NC}"
fi

# Check for any ports still in use
PORTS_STILL_IN_USE=false
for port in "${PORTS[@]}"; do
    port_pids=$(check_port "$port")
    if [[ -n "$port_pids" ]]; then
        if [[ "$PORTS_STILL_IN_USE" == false ]]; then
            echo -e "${YELLOW}⚠️  Ports still in use:${NC}"
            PORTS_STILL_IN_USE=true
        fi
        echo -e "  Port $port: PIDs $port_pids"
        ALL_CLEAR=false
    fi
done

if [[ "$PORTS_STILL_IN_USE" == false ]]; then
    echo -e "${GREEN}✓ All FEAGI ports are free${NC}"
fi

echo -e "${BLUE}═══════════════════════════════════════════════════════════════════${NC}"
echo ""

# Final status
if [[ "$ALL_CLEAR" == true ]]; then
    echo -e "${GREEN}✓ Cleanup complete - ready to start FEAGI${NC}"
    exit 0
else
    echo -e "${YELLOW}⚠️  Some processes/ports were not cleaned up${NC}"
    echo -e "${YELLOW}You may need to run with --force and --yes flags:${NC}"
    echo -e "  ${BLUE}$0 --force --yes${NC}"
    exit 1
fi
