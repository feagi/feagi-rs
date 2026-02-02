#!/usr/bin/env bash
# FEAGI-RS - Rust Build Artifacts Cleanup Script
# 
# This script cleans up accumulated Rust build artifacts from the target directory.
# Rust's incremental compilation can accumulate 20-30GB of build artifacts over time.
# This script provides options for full or selective cleanup.
#
# Usage:
#   ./clean-rust-build.sh [option]
#
# Options:
#   --all           Remove entire target directory (default, frees most space)
#   --debug         Remove only debug builds (keeps release builds)
#   --incremental   Remove only incremental compilation cache
#   --stats         Show current disk usage statistics
#   --help          Show this help message

set -e  # Exit on error

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TARGET_DIR="$PROJECT_ROOT/target"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to get directory size
get_size() {
    local dir="$1"
    if [ -d "$dir" ]; then
        du -sh "$dir" 2>/dev/null | cut -f1
    else
        echo "0B"
    fi
}

# Function to show statistics
show_stats() {
    print_info "Current Rust build artifacts disk usage:"
    echo ""
    
    if [ ! -d "$TARGET_DIR" ]; then
        print_warning "Target directory does not exist: $TARGET_DIR"
        return
    fi
    
    echo "  Total:        $(get_size "$TARGET_DIR")"
    echo "  Debug:        $(get_size "$TARGET_DIR/debug")"
    echo "  Release:      $(get_size "$TARGET_DIR/release")"
    echo "  Incremental:  $(get_size "$TARGET_DIR/debug/incremental") + $(get_size "$TARGET_DIR/release/incremental")"
    echo ""
    
    if [ -d "$TARGET_DIR/debug/deps" ]; then
        local dep_count=$(find "$TARGET_DIR/debug/deps" -type f 2>/dev/null | wc -l | tr -d ' ')
        echo "  Debug deps files: $dep_count"
    fi
    
    if [ -d "$TARGET_DIR/debug/incremental" ]; then
        local inc_count=$(find "$TARGET_DIR/debug/incremental" -type d -name "s-*" 2>/dev/null | wc -l | tr -d ' ')
        echo "  Incremental sessions: $inc_count"
    fi
    echo ""
}

# Function to clean all build artifacts
clean_all() {
    print_info "Cleaning all Rust build artifacts..."
    show_stats
    
    if [ ! -d "$TARGET_DIR" ]; then
        print_warning "Target directory does not exist. Nothing to clean."
        return
    fi
    
    read -p "This will remove the entire target directory. Continue? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        print_warning "Cleanup cancelled."
        return
    fi
    
    cd "$PROJECT_ROOT"
    cargo clean
    
    print_success "All build artifacts cleaned!"
    print_info "Next build will be from scratch (slower but clean)."
}

# Function to clean only debug builds
clean_debug() {
    print_info "Cleaning debug build artifacts..."
    
    if [ ! -d "$TARGET_DIR/debug" ]; then
        print_warning "Debug directory does not exist. Nothing to clean."
        return
    fi
    
    local size=$(get_size "$TARGET_DIR/debug")
    print_info "Debug directory size: $size"
    
    read -p "Remove debug directory? (y/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        print_warning "Cleanup cancelled."
        return
    fi
    
    rm -rf "$TARGET_DIR/debug"
    
    print_success "Debug build artifacts cleaned!"
    print_info "Release builds are preserved."
}

# Function to clean only incremental compilation cache
clean_incremental() {
    print_info "Cleaning incremental compilation cache..."
    
    local cleaned=0
    
    if [ -d "$TARGET_DIR/debug/incremental" ]; then
        local size=$(get_size "$TARGET_DIR/debug/incremental")
        print_info "Debug incremental cache: $size"
        rm -rf "$TARGET_DIR/debug/incremental"
        cleaned=1
    fi
    
    if [ -d "$TARGET_DIR/release/incremental" ]; then
        local size=$(get_size "$TARGET_DIR/release/incremental")
        print_info "Release incremental cache: $size"
        rm -rf "$TARGET_DIR/release/incremental"
        cleaned=1
    fi
    
    if [ $cleaned -eq 0 ]; then
        print_warning "No incremental cache found."
    else
        print_success "Incremental compilation cache cleaned!"
        print_info "Next build will rebuild incrementally but start fresh."
    fi
}

# Function to show help
show_help() {
    cat << EOF
FEAGI-RS - Rust Build Artifacts Cleanup Script

This script cleans up accumulated Rust build artifacts that can grow to 20-30GB.

Usage:
    ./clean-rust-build.sh [option]

Options:
    --all           Remove entire target directory (frees most space, ~20-30GB)
    --debug         Remove only debug builds (frees ~20-25GB, keeps release)
    --incremental   Remove only incremental cache (frees ~3-5GB)
    --stats         Show current disk usage statistics
    --help          Show this help message

Examples:
    # Show current disk usage
    ./clean-rust-build.sh --stats

    # Clean everything (recommended monthly)
    ./clean-rust-build.sh --all

    # Clean only debug builds (keep release for distribution)
    ./clean-rust-build.sh --debug

    # Clean only incremental cache (minimal impact on next build)
    ./clean-rust-build.sh --incremental

Notes:
    - Debug builds include full debugging symbols and are much larger
    - Release builds are optimized and smaller
    - Incremental compilation cache speeds up rebuilds but accumulates over time
    - After cleanup, the next build will take longer but start fresh
    - This script only cleans build artifacts, not running processes
    - For cleaning running FEAGI processes, use cleanup_feagi.sh

For more information, see: feagi-rs/docs/
EOF
}

# Main script logic
main() {
    local option="${1:---all}"
    
    case "$option" in
        --all)
            clean_all
            ;;
        --debug)
            clean_debug
            ;;
        --incremental)
            clean_incremental
            ;;
        --stats)
            show_stats
            ;;
        --help|-h)
            show_help
            ;;
        *)
            print_error "Unknown option: $option"
            echo ""
            show_help
            exit 1
            ;;
    esac
}

main "$@"
