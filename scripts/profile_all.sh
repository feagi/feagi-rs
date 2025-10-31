#!/bin/bash
# Profile all aspects of FEAGI: memory, CPU, storage, and benchmarks
# This script should be run from the feagi/ directory

set -e

echo "════════════════════════════════════════════════════════════════"
echo "  FEAGI Performance Profiling Suite"
echo "  Version: $(cargo pkgid | cut -d# -f2 | cut -d: -f2)"
echo "  Date: $(date)"
echo "════════════════════════════════════════════════════════════════"
echo ""

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ]; then
    echo "❌ Error: Must be run from the feagi/ directory"
    exit 1
fi

# Parse arguments
SKIP_BUILD=0
SKIP_TESTS=0
SKIP_BENCH=0
SAVE_HISTORY=0

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-build)
            SKIP_BUILD=1
            shift
            ;;
        --skip-tests)
            SKIP_TESTS=1
            shift
            ;;
        --skip-bench)
            SKIP_BENCH=1
            shift
            ;;
        --save-history)
            SAVE_HISTORY=1
            shift
            ;;
        --help)
            echo "Usage: $0 [options]"
            echo ""
            echo "Options:"
            echo "  --skip-build    Skip cargo build step"
            echo "  --skip-tests    Skip profiling tests"
            echo "  --skip-bench    Skip benchmarks"
            echo "  --save-history  Save results to profiling_history/"
            echo "  --help          Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Step 1: Build release binary
if [ $SKIP_BUILD -eq 0 ]; then
    echo "📦 Step 1/4: Building release binary..."
    cargo build --release
    echo "✓ Build complete"
    echo ""
else
    echo "⏭️  Skipping build step"
    echo ""
fi

# Step 2: Run profiling tests
if [ $SKIP_TESTS -eq 0 ]; then
    echo "🔍 Step 2/4: Running profiling tests..."
    echo ""
    
    echo "  [1/3] Memory Profiling..."
    cargo test --release --test profile_memory -- --nocapture
    echo ""
    
    echo "  [2/3] CPU Profiling..."
    cargo test --release --test profile_cpu -- --nocapture
    echo ""
    
    echo "  [3/3] Storage Profiling..."
    cargo test --release --test profile_storage -- --nocapture
    echo ""
    
    echo "✓ Profiling tests complete"
    echo ""
else
    echo "⏭️  Skipping profiling tests"
    echo ""
fi

# Step 3: Run benchmarks
if [ $SKIP_BENCH -eq 0 ]; then
    echo "⚡ Step 3/4: Running benchmarks..."
    cargo bench
    echo ""
    echo "✓ Benchmarks complete"
    echo ""
else
    echo "⏭️  Skipping benchmarks"
    echo ""
fi

# Step 4: Save results to history (if requested)
if [ $SAVE_HISTORY -eq 1 ]; then
    echo "💾 Step 4/4: Saving results to history..."
    
    VERSION=$(cargo pkgid | cut -d# -f2 | cut -d: -f2)
    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    HISTORY_DIR="profiling_history/${VERSION}_${TIMESTAMP}"
    
    mkdir -p "$HISTORY_DIR"
    
    # Copy JSON results
    if [ -f "target/profile_memory_results.json" ]; then
        cp target/profile_memory_results.json "$HISTORY_DIR/"
    fi
    if [ -f "target/profile_cpu_results.json" ]; then
        cp target/profile_cpu_results.json "$HISTORY_DIR/"
    fi
    if [ -f "target/profile_storage_results.json" ]; then
        cp target/profile_storage_results.json "$HISTORY_DIR/"
    fi
    
    # Copy benchmark results
    if [ -d "target/criterion" ]; then
        mkdir -p "$HISTORY_DIR/criterion"
        cp -r target/criterion/report "$HISTORY_DIR/criterion/" 2>/dev/null || true
    fi
    
    # Create summary file
    cat > "$HISTORY_DIR/SUMMARY.txt" << EOF
FEAGI Performance Profile Summary
==================================

Version: $VERSION
Date: $(date)
Host: $(hostname)
OS: $(uname -s) $(uname -r)
Rust: $(rustc --version)

Files:
- profile_memory_results.json
- profile_cpu_results.json
- profile_storage_results.json
- criterion/ (benchmark reports)

To view benchmark reports:
  open $HISTORY_DIR/criterion/report/index.html
EOF
    
    echo "✓ Results saved to: $HISTORY_DIR"
    echo ""
else
    echo "⏭️  Not saving history (use --save-history to enable)"
    echo ""
fi

# Print summary
echo "════════════════════════════════════════════════════════════════"
echo "  Profiling Complete!"
echo "════════════════════════════════════════════════════════════════"
echo ""
echo "Results available at:"
echo "  • Memory:     target/profile_memory_results.json"
echo "  • CPU:        target/profile_cpu_results.json"
echo "  • Storage:    target/profile_storage_results.json"
echo "  • Benchmarks: target/criterion/report/index.html"
echo ""
echo "View benchmark reports:"
echo "  open target/criterion/report/index.html"
echo ""
if [ $SAVE_HISTORY -eq 1 ]; then
    echo "Historical data saved to:"
    echo "  $HISTORY_DIR"
    echo ""
fi
echo "See docs/PROFILING_GUIDE.md for detailed information"
echo ""

