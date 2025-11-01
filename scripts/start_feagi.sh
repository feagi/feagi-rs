#!/bin/bash
# Copyright 2025 Neuraville Inc.
# Licensed under the Apache License, Version 2.0
#
# FEAGI Quick Start Script (Mac/Linux)
# 
# This script cleans up old instances and starts a fresh FEAGI server
# Usage: ./start_feagi.sh [--debug-all] [--genome path/to/genome.json] [--yes]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔═══════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║                    FEAGI Quick Start                             ║${NC}"
echo -e "${BLUE}╚═══════════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Parse command line arguments
DEBUG_ALL=false
GENOME_PATH=""
AUTO_YES=false
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --debug-all)
            DEBUG_ALL=true
            shift
            ;;
        --genome|-g)
            GENOME_PATH="$2"
            shift 2
            ;;
        --yes|-y)
            AUTO_YES=true
            shift
            ;;
        *)
            EXTRA_ARGS+=("$1")
            shift
            ;;
    esac
done

# Step 1: Cleanup
echo -e "${BLUE}[Step 1/3] Cleaning up old FEAGI instances...${NC}"
if [[ -f "$SCRIPT_DIR/cleanup_feagi.sh" ]]; then
    if [[ "$AUTO_YES" == true ]]; then
        "$SCRIPT_DIR/cleanup_feagi.sh" --yes
    else
        "$SCRIPT_DIR/cleanup_feagi.sh"
    fi
    
    if [[ $? -eq 0 ]]; then
        echo -e "${GREEN}✓ Cleanup complete${NC}"
    else
        echo -e "${YELLOW}⚠️  Cleanup had warnings, but continuing...${NC}"
    fi
    echo ""
else
    echo -e "${YELLOW}⚠️  cleanup_feagi.sh not found, skipping cleanup${NC}"
    echo ""
fi

# Step 2: Build (if needed)
echo -e "${BLUE}[Step 2/3] Building FEAGI...${NC}"
cd "$PROJECT_ROOT"

if cargo build --release --quiet 2>&1 | grep -q "error"; then
    echo -e "${RED}✗ Build failed${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Build complete${NC}"
echo ""

# Step 3: Start FEAGI
echo -e "${BLUE}[Step 3/3] Starting FEAGI server...${NC}"
echo ""

# Build command
CMD="cargo run --release --"

# Add config
CMD="$CMD --config ./feagi_configuration.toml"

# Add debug flag if requested
if [[ "$DEBUG_ALL" == true ]]; then
    CMD="$CMD --debug-all"
fi

# Add genome if provided
if [[ -n "$GENOME_PATH" ]]; then
    if [[ ! -f "$GENOME_PATH" ]]; then
        echo -e "${RED}✗ Genome file not found: $GENOME_PATH${NC}"
        exit 1
    fi
    CMD="$CMD --genome $GENOME_PATH"
fi

# Add any extra arguments
for arg in "${EXTRA_ARGS[@]}"; do
    CMD="$CMD $arg"
done

echo -e "${GREEN}Starting FEAGI with command:${NC}"
echo -e "  ${BLUE}$CMD${NC}"
echo ""
echo -e "${YELLOW}Press Ctrl+C to stop FEAGI${NC}"
echo ""

# Execute
exec $CMD

