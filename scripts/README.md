# FEAGI Utility Scripts

This directory contains utility scripts for managing FEAGI server instances.

## Available Scripts

### Cleanup Scripts

#### Mac/Linux: `cleanup_feagi.sh`
```bash
./cleanup_feagi.sh                 # Interactive mode (asks permission)
./cleanup_feagi.sh --yes           # Auto-approve all cleanups
./cleanup_feagi.sh --force         # Use SIGKILL immediately
./cleanup_feagi.sh --force --yes   # Force kill with auto-approve
```

#### Windows: `cleanup_feagi.ps1`
```powershell
.\cleanup_feagi.ps1                # Interactive mode (asks permission)
.\cleanup_feagi.ps1 -Yes           # Auto-approve all cleanups
.\cleanup_feagi.ps1 -Force         # Immediate termination
.\cleanup_feagi.ps1 -Force -Yes    # Force kill with auto-approve
```

**What it does:**
1. Finds all running FEAGI processes (Rust and Python)
2. **Asks permission before killing** each group of processes
3. Checks FEAGI ports (8000, 5555-5564) for occupancy
4. **Asks permission before killing** port-occupying processes
5. Verifies all processes terminated and ports released

**Safe defaults:**
- Uses graceful SIGTERM first (5 second wait)
- Falls back to SIGKILL only if needed
- Skips non-FEAGI processes on ports
- Always asks permission unless `--yes` flag used

---

### Quick Start Scripts

#### Mac/Linux: `start_feagi.sh`
```bash
./start_feagi.sh                           # Clean start (interactive cleanup)
./start_feagi.sh --yes                     # Auto-approve cleanup
./start_feagi.sh --debug-all               # Start with debug logging
./start_feagi.sh --genome genome.json      # Start with genome
./start_feagi.sh --debug-all --genome genome.json --yes
```

#### Windows: `start_feagi.ps1`
```powershell
.\start_feagi.ps1                          # Clean start (interactive cleanup)
.\start_feagi.ps1 -Yes                     # Auto-approve cleanup
.\start_feagi.ps1 -DebugAll                # Start with debug logging
.\start_feagi.ps1 -Genome genome.json      # Start with genome
.\start_feagi.ps1 -DebugAll -Genome genome.json -Yes
```

**What it does:**
1. Runs cleanup (with user permission)
2. Builds FEAGI (release mode)
3. Starts FEAGI with specified options

**Passes through extra arguments:**
```bash
./start_feagi.sh --api-port 9000           # Override API port
./start_feagi.sh --burst-hz 5000           # Override burst frequency
```

---

## Usage Examples

### Daily Development Workflow (Mac/Linux)

```bash
# Morning: Start fresh FEAGI
cd /Users/nadji/code/FEAGI-2.0/feagi/scripts
./start_feagi.sh --debug-all --yes

# Load genome via Swagger at http://localhost:8000/swagger-ui/

# Evening: Clean shutdown (Ctrl+C), then cleanup stragglers
./cleanup_feagi.sh --yes
```

### Daily Development Workflow (Windows)

```powershell
# Morning: Start fresh FEAGI
cd C:\code\FEAGI-2.0\feagi\scripts
.\start_feagi.ps1 -DebugAll -Yes

# Load genome via Swagger at http://localhost:8000/swagger-ui/

# Evening: Clean shutdown (Ctrl+C), then cleanup stragglers
.\cleanup_feagi.ps1 -Yes
```

### CI/CD or Automated Testing

```bash
# Non-interactive cleanup
./cleanup_feagi.sh --force --yes

# Start with genome for testing
./start_feagi.sh --genome ../genomes/test_genome.json --yes
```

---

## Port Reference

Scripts check and clean up these ports:

| Port | Purpose | Protocol |
|------|---------|----------|
| 8000 | HTTP REST API | HTTP |
| 5555 | ZMQ REQ/REP | ZMQ |
| 5556 | ZMQ PUB/SUB | ZMQ |
| 5557 | ZMQ PUSH/PULL | ZMQ |
| 5558 | Sensory input | ZMQ PULL |
| 5562 | Visualization (FCL) | ZMQ PUB |
| 5563 | ZMQ REST (agent reg) | ZMQ REP |
| 5564 | Motor output | ZMQ PUB |

---

## Troubleshooting

### "Cleanup incomplete" error

If cleanup fails, check what's blocking:
```bash
# Mac/Linux
lsof -i :8000
lsof -i :5563

# Windows
Get-NetTCPConnection -LocalPort 8000
Get-NetTCPConnection -LocalPort 5563
```

Then either:
1. Manually kill: `kill -9 <PID>` (Mac/Linux) or `Stop-Process -Id <PID> -Force` (Windows)
2. Run cleanup with force: `./cleanup_feagi.sh --force --yes`

### Permission denied

```bash
# Mac/Linux
chmod +x cleanup_feagi.sh
chmod +x start_feagi.sh

# Windows: Run as Administrator or adjust execution policy
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
```

---

## Safety Features

✅ **Interactive permission prompts** - won't kill processes without asking  
✅ **Graceful shutdown first** - tries SIGTERM before SIGKILL  
✅ **Non-FEAGI process detection** - skips unrelated processes  
✅ **Clear status reporting** - shows what was cleaned up  
✅ **Platform-specific implementations** - proper for each OS  

**Use `--yes` flag only when you're sure you want to kill all FEAGI processes!**

