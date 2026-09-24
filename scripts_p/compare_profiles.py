#!/usr/bin/env python3
"""
Compare profiling results between two builds to track performance changes.

Usage:
    ./scripts/compare_profiles.py <old_dir> <new_dir>

Example:
    ./scripts/compare_profiles.py profiling_history/2.0.0_20250101 profiling_history/2.0.1_20250102
"""

import json
import sys
from pathlib import Path
from typing import Dict, Any, List, Tuple


class Color:
    """ANSI color codes for terminal output."""
    RED = '\033[91m'
    GREEN = '\033[92m'
    YELLOW = '\033[93m'
    BLUE = '\033[94m'
    BOLD = '\033[1m'
    END = '\033[0m'


def load_json(path: Path) -> Dict[str, Any]:
    """Load JSON file safely."""
    try:
        with open(path) as f:
            return json.load(f)
    except FileNotFoundError:
        print(f"Warning: {path} not found")
        return {}
    except json.JSONDecodeError:
        print(f"Warning: {path} is not valid JSON")
        return {}


def format_delta(value: float, is_percent: bool = True, reverse: bool = False) -> str:
    """Format a delta value with color coding."""
    if value == 0:
        return f"{Color.BLUE}  0.0%{Color.END}" if is_percent else f"{Color.BLUE}  0{Color.END}"
    
    # For memory/time, lower is better (reverse=False)
    # For ops/sec, higher is better (reverse=True)
    is_good = (value < 0) if not reverse else (value > 0)
    color = Color.GREEN if is_good else Color.RED
    
    sign = "+" if value > 0 else ""
    if is_percent:
        return f"{color}{sign}{value:.1f}%{Color.END}"
    else:
        return f"{color}{sign}{value:.2f}{Color.END}"


def compare_memory(old_data: Dict, new_data: Dict) -> None:
    """Compare memory profiling results."""
    print(f"\n{Color.BOLD}Memory Profile Comparison{Color.END}")
    print("=" * 80)
    
    if not old_data or not new_data:
        print("Insufficient data for comparison")
        return
    
    categories = [
        ('npu_memory', 'NPU Memory'),
        ('neuron_creation_memory', 'Neuron Creation Memory'),
        ('connectome_manager_memory', 'ConnectomeManager Memory'),
    ]
    
    for key, title in categories:
        if key not in old_data or key not in new_data:
            continue
        
        print(f"\n{Color.BOLD}{title}{Color.END}")
        print(f"{'Component':<35} {'Old (KB)':>12} {'New (KB)':>12} {'Delta':>12}")
        print("-" * 80)
        
        old_components = {c['component']: c for c in old_data[key]}
        new_components = {c['component']: c for c in new_data[key]}
        
        for component in sorted(new_components.keys()):
            if component not in old_components:
                continue
            
            old_rss = old_components[component]['rss_kb']
            new_rss = new_components[component]['rss_kb']
            
            if old_rss > 0:
                delta_pct = ((new_rss - old_rss) / old_rss) * 100
            else:
                delta_pct = 0
            
            delta_str = format_delta(delta_pct)
            
            print(f"{component:<35} {old_rss:>12,} {new_rss:>12,} {delta_str:>20}")


def compare_cpu(old_data: Dict, new_data: Dict) -> None:
    """Compare CPU profiling results."""
    print(f"\n{Color.BOLD}CPU Profile Comparison{Color.END}")
    print("=" * 90)
    
    if not old_data or not new_data or 'metrics' not in old_data or 'metrics' not in new_data:
        print("Insufficient data for comparison")
        return
    
    old_metrics = {m['operation']: m for m in old_data['metrics']}
    new_metrics = {m['operation']: m for m in new_data['metrics']}
    
    print(f"{'Operation':<40} {'Old (ops/s)':>15} {'New (ops/s)':>15} {'Delta':>12}")
    print("-" * 90)
    
    for operation in sorted(new_metrics.keys()):
        if operation not in old_metrics:
            continue
        
        old_ops = old_metrics[operation]['ops_per_second']
        new_ops = new_metrics[operation]['ops_per_second']
        
        if old_ops > 0:
            delta_pct = ((new_ops - old_ops) / old_ops) * 100
        else:
            delta_pct = 0
        
        # For ops/sec, higher is better
        delta_str = format_delta(delta_pct, reverse=True)
        
        print(f"{operation:<40} {old_ops:>15.2f} {new_ops:>15.2f} {delta_str:>20}")


def compare_storage(old_data: Dict, new_data: Dict) -> None:
    """Compare storage profiling results."""
    print(f"\n{Color.BOLD}Storage Profile Comparison{Color.END}")
    print("=" * 80)
    
    if not old_data or not new_data or 'binaries' not in old_data or 'binaries' not in new_data:
        print("Insufficient data for comparison")
        return
    
    old_binaries = {f"{b['name']}_{b['profile']}": b for b in old_data['binaries']}
    new_binaries = {f"{b['name']}_{b['profile']}": b for b in new_data['binaries']}
    
    print(f"{'Binary':<30} {'Old (MB)':>12} {'New (MB)':>12} {'Delta':>12}")
    print("-" * 80)
    
    for key in sorted(new_binaries.keys()):
        if key not in old_binaries:
            continue
        
        old_size = old_binaries[key]['size_mb']
        new_size = new_binaries[key]['size_mb']
        
        if old_size > 0:
            delta_pct = ((new_size - old_size) / old_size) * 100
        else:
            delta_pct = 0
        
        delta_str = format_delta(delta_pct)
        
        binary_name = new_binaries[key]['name']
        profile = new_binaries[key]['profile']
        
        print(f"{binary_name:<20} ({profile:<7}) {old_size:>12.2f} {new_size:>12.2f} {delta_str:>20}")


def print_summary(old_dir: Path, new_dir: Path) -> None:
    """Print comparison summary."""
    print(f"\n{Color.BOLD}{'=' * 80}{Color.END}")
    print(f"{Color.BOLD}Profiling Comparison Summary{Color.END}")
    print(f"{Color.BOLD}{'=' * 80}{Color.END}")
    print(f"\nOld: {old_dir}")
    print(f"New: {new_dir}")
    print("\nLegend:")
    print(f"  {Color.GREEN}Green{Color.END} = Improvement (lower memory/time, higher ops/sec)")
    print(f"  {Color.RED}Red{Color.END}   = Regression (higher memory/time, lower ops/sec)")
    print(f"  {Color.BLUE}Blue{Color.END}  = No change")
    print()


def main():
    """Main entry point."""
    if len(sys.argv) != 3:
        print("Usage: compare_profiles.py <old_dir> <new_dir>")
        print("\nExample:")
        print("  ./scripts/compare_profiles.py \\")
        print("      profiling_history/2.0.0_20250101 \\")
        print("      profiling_history/2.0.1_20250102")
        sys.exit(1)
    
    old_dir = Path(sys.argv[1])
    new_dir = Path(sys.argv[2])
    
    if not old_dir.exists():
        print(f"Error: {old_dir} does not exist")
        sys.exit(1)
    
    if not new_dir.exists():
        print(f"Error: {new_dir} does not exist")
        sys.exit(1)
    
    print_summary(old_dir, new_dir)
    
    # Load data
    old_memory = load_json(old_dir / "profile_memory_results.json")
    new_memory = load_json(new_dir / "profile_memory_results.json")
    
    old_cpu = load_json(old_dir / "profile_cpu_results.json")
    new_cpu = load_json(new_dir / "profile_cpu_results.json")
    
    old_storage = load_json(old_dir / "profile_storage_results.json")
    new_storage = load_json(new_dir / "profile_storage_results.json")
    
    # Compare
    compare_memory(old_memory, new_memory)
    compare_cpu(old_cpu, new_cpu)
    compare_storage(old_storage, new_storage)
    
    print(f"\n{Color.BOLD}{'=' * 80}{Color.END}\n")


if __name__ == '__main__':
    main()




