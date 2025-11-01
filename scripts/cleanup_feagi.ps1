# Copyright 2025 Neuraville Inc.
# Licensed under the Apache License, Version 2.0
#
# FEAGI Cleanup Script (Windows PowerShell)
# 
# This script safely terminates all running FEAGI instances and releases ports.
# Usage: .\cleanup_feagi.ps1 [-Force] [-Yes]

param(
    [switch]$Force,
    [switch]$Yes
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir

# FEAGI ports to check/cleanup
$Ports = @(8000, 5555, 5556, 5557, 5558, 5562, 5563, 5564)

Write-Host "╔═══════════════════════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║                    FEAGI Cleanup Utility                         ║" -ForegroundColor Cyan
Write-Host "╚═══════════════════════════════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

if ($Force) {
    Write-Host "⚠️  Force mode enabled - will terminate processes immediately" -ForegroundColor Yellow
    Write-Host ""
}

# Function to ask for permission
function Ask-Permission {
    param([string]$Message)
    
    if ($Yes) {
        return $true
    }
    
    Write-Host $Message -ForegroundColor Yellow
    $response = Read-Host "Continue? [y/N]"
    return $response -match '^[Yy]$'
}

# Function to kill a process
function Kill-FeagiProcess {
    param(
        [int]$ProcessId,
        [string]$ProcessName
    )
    
    try {
        $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        if (-not $process) {
            Write-Host "  Process $ProcessId already terminated" -ForegroundColor Yellow
            return $true
        }
        
        if ($Force) {
            Write-Host "  Sending immediate termination to $ProcessId ($ProcessName)" -ForegroundColor Red
            Stop-Process -Id $ProcessId -Force -ErrorAction Stop
            Start-Sleep -Milliseconds 500
        } else {
            Write-Host "  Sending graceful termination to $ProcessId ($ProcessName)" -ForegroundColor Yellow
            Stop-Process -Id $ProcessId -ErrorAction Stop
            
            # Wait up to 5 seconds for graceful shutdown
            $waited = 0
            while ($waited -lt 5000) {
                $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
                if (-not $process) {
                    Write-Host "  ✓ Process $ProcessId terminated gracefully" -ForegroundColor Green
                    return $true
                }
                Start-Sleep -Milliseconds 500
                $waited += 500
            }
            
            # If still running, force kill
            $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
            if ($process) {
                Write-Host "  Process $ProcessId did not respond, forcing termination" -ForegroundColor Red
                Stop-Process -Id $ProcessId -Force -ErrorAction Stop
                Start-Sleep -Milliseconds 500
            }
        }
        
        # Final check
        $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        if ($process) {
            Write-Host "  ✗ Failed to terminate process $ProcessId" -ForegroundColor Red
            return $false
        } else {
            Write-Host "  ✓ Process $ProcessId terminated" -ForegroundColor Green
            return $true
        }
    } catch {
        Write-Host "  Error terminating process $ProcessId : $_" -ForegroundColor Red
        return $false
    }
}

# Step 1: Find and kill FEAGI processes by name
Write-Host "[1/3] Checking for FEAGI processes..." -ForegroundColor Cyan
$FeagiProcesses = Get-Process | Where-Object { $_.ProcessName -match "feagi" -or $_.Path -match "feagi" }

if ($FeagiProcesses.Count -eq 0) {
    Write-Host "  ✓ No FEAGI processes found" -ForegroundColor Green
    Write-Host ""
} else {
    Write-Host "  Found FEAGI processes:" -ForegroundColor Yellow
    foreach ($proc in $FeagiProcesses) {
        $cmdLine = (Get-CimInstance Win32_Process -Filter "ProcessId = $($proc.Id)" -ErrorAction SilentlyContinue).CommandLine
        Write-Host "    PID $($proc.Id): $($proc.ProcessName) - $cmdLine" -ForegroundColor Yellow
    }
    Write-Host ""
    
    if (Ask-Permission "  Kill these FEAGI processes?") {
        foreach ($proc in $FeagiProcesses) {
            Kill-FeagiProcess -ProcessId $proc.Id -ProcessName $proc.ProcessName
        }
    } else {
        Write-Host "Skipping FEAGI process cleanup" -ForegroundColor Yellow
    }
    Write-Host ""
}

# Step 2: Check for cargo processes running FEAGI
Write-Host "[2/3] Checking for cargo build/run processes..." -ForegroundColor Cyan
$CargoProcesses = Get-Process | Where-Object { 
    ($_.ProcessName -match "cargo" -or $_.ProcessName -match "rustc") -and 
    ($_.Path -match "feagi" -or (Get-CimInstance Win32_Process -Filter "ProcessId = $($_.Id)" -ErrorAction SilentlyContinue).CommandLine -match "feagi")
}

if ($CargoProcesses.Count -eq 0) {
    Write-Host "  ✓ No cargo FEAGI processes found" -ForegroundColor Green
    Write-Host ""
} else {
    Write-Host "  Found cargo processes:" -ForegroundColor Yellow
    foreach ($proc in $CargoProcesses) {
        $cmdLine = (Get-CimInstance Win32_Process -Filter "ProcessId = $($proc.Id)" -ErrorAction SilentlyContinue).CommandLine
        Write-Host "    PID $($proc.Id): $($proc.ProcessName) - $cmdLine" -ForegroundColor Yellow
    }
    Write-Host ""
    
    if (Ask-Permission "  Kill these cargo processes?") {
        foreach ($proc in $CargoProcesses) {
            Kill-FeagiProcess -ProcessId $proc.Id -ProcessName $proc.ProcessName
        }
    } else {
        Write-Host "Skipping cargo process cleanup" -ForegroundColor Yellow
    }
    Write-Host ""
}

# Step 3: Check and clean up specific ports
Write-Host "[3/3] Checking FEAGI ports..." -ForegroundColor Cyan
$PortsInUse = $false
$PortPidsToKill = @()

foreach ($port in $Ports) {
    $connections = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
    
    if ($connections) {
        $PortsInUse = $true
        Write-Host "  Port $port is in use:" -ForegroundColor Yellow
        
        foreach ($conn in $connections) {
            $pid = $conn.OwningProcess
            $process = Get-Process -Id $pid -ErrorAction SilentlyContinue
            
            if ($process) {
                $cmdLine = (Get-CimInstance Win32_Process -Filter "ProcessId = $pid" -ErrorAction SilentlyContinue).CommandLine
                Write-Host "    PID ${pid}: $($process.ProcessName) - $cmdLine" -ForegroundColor Yellow
                
                # Only add FEAGI-related processes
                if ($process.ProcessName -match "feagi|cargo|python|rustc" -or $cmdLine -match "feagi") {
                    $PortPidsToKill += $pid
                } else {
                    Write-Host "    (Non-FEAGI process, will skip)" -ForegroundColor Blue
                }
            }
        }
    }
}

if (-not $PortsInUse) {
    Write-Host "  ✓ All FEAGI ports are free" -ForegroundColor Green
    Write-Host ""
} else {
    Write-Host ""
    if ($PortPidsToKill.Count -gt 0) {
        $uniquePids = $PortPidsToKill | Select-Object -Unique
        
        if (Ask-Permission "  Kill processes occupying FEAGI ports?") {
            foreach ($pid in $uniquePids) {
                $process = Get-Process -Id $pid -ErrorAction SilentlyContinue
                if ($process) {
                    Kill-FeagiProcess -ProcessId $pid -ProcessName $process.ProcessName
                }
            }
        } else {
            Write-Host "Skipping port cleanup" -ForegroundColor Yellow
        }
    }
    Write-Host ""
}

# Final verification
Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host "Cleanup Summary" -ForegroundColor Cyan
Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Cyan

$AllClear = $true

# Check for remaining FEAGI processes
$RemainingFeagi = Get-Process | Where-Object { $_.ProcessName -match "feagi" -or $_.Path -match "feagi" }
if ($RemainingFeagi.Count -gt 0) {
    Write-Host "⚠️  FEAGI processes still running:" -ForegroundColor Yellow
    foreach ($proc in $RemainingFeagi) {
        Write-Host "  PID $($proc.Id): $($proc.ProcessName)" -ForegroundColor Yellow
    }
    $AllClear = $false
} else {
    Write-Host "✓ No FEAGI processes running" -ForegroundColor Green
}

# Check for ports still in use
$PortsStillInUse = $false
foreach ($port in $Ports) {
    $connections = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
    if ($connections) {
        if (-not $PortsStillInUse) {
            Write-Host "⚠️  Ports still in use:" -ForegroundColor Yellow
            $PortsStillInUse = $true
        }
        $pids = ($connections | ForEach-Object { $_.OwningProcess }) -join ", "
        Write-Host "  Port ${port}: PIDs $pids" -ForegroundColor Yellow
        $AllClear = $false
    }
}

if (-not $PortsStillInUse) {
    Write-Host "✓ All FEAGI ports are free" -ForegroundColor Green
}

Write-Host "═══════════════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host ""

# Final status
if ($AllClear) {
    Write-Host "╔═══════════════════════════════════════════════════════════════════╗" -ForegroundColor Green
    Write-Host "║                     ✓ CLEANUP SUCCESSFUL                         ║" -ForegroundColor Green
    Write-Host "║                All FEAGI instances terminated                    ║" -ForegroundColor Green
    Write-Host "║                   All ports released                             ║" -ForegroundColor Green
    Write-Host "╚═══════════════════════════════════════════════════════════════════╝" -ForegroundColor Green
    Write-Host ""
    Write-Host "You can now start FEAGI:" -ForegroundColor Cyan
    Write-Host "  cd $ProjectRoot" -ForegroundColor Green
    Write-Host "  cargo run --release -- --config .\feagi_configuration.toml" -ForegroundColor Green
    Write-Host ""
    exit 0
} else {
    Write-Host "╔═══════════════════════════════════════════════════════════════════╗" -ForegroundColor Red
    Write-Host "║                    ✗ CLEANUP INCOMPLETE                          ║" -ForegroundColor Red
    Write-Host "║           Some processes or ports still occupied                 ║" -ForegroundColor Red
    Write-Host "╚═══════════════════════════════════════════════════════════════════╝" -ForegroundColor Red
    Write-Host ""
    Write-Host "Try running with -Force and -Yes flags:" -ForegroundColor Yellow
    Write-Host "  .\cleanup_feagi.ps1 -Force -Yes" -ForegroundColor Yellow
    Write-Host ""
    exit 1
}

