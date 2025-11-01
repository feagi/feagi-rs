# Copyright 2025 Neuraville Inc.
# Licensed under the Apache License, Version 2.0
#
# FEAGI Quick Start Script (Windows PowerShell)
# 
# This script cleans up old instances and starts a fresh FEAGI server
# Usage: .\start_feagi.ps1 [-DebugAll] [-Genome <path>] [-Yes]

param(
    [switch]$DebugAll,
    [string]$Genome = "",
    [switch]$Yes,
    [string[]]$ExtraArgs = @()
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir

Write-Host "╔═══════════════════════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║                    FEAGI Quick Start                             ║" -ForegroundColor Cyan
Write-Host "╚═══════════════════════════════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

# Step 1: Cleanup
Write-Host "[Step 1/3] Cleaning up old FEAGI instances..." -ForegroundColor Cyan
$cleanupScript = Join-Path $ScriptDir "cleanup_feagi.ps1"

if (Test-Path $cleanupScript) {
    if ($Yes) {
        & $cleanupScript -Yes
    } else {
        & $cleanupScript
    }
    
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✓ Cleanup complete" -ForegroundColor Green
    } else {
        Write-Host "⚠️  Cleanup had warnings, but continuing..." -ForegroundColor Yellow
    }
    Write-Host ""
} else {
    Write-Host "⚠️  cleanup_feagi.ps1 not found, skipping cleanup" -ForegroundColor Yellow
    Write-Host ""
}

# Step 2: Build
Write-Host "[Step 2/3] Building FEAGI..." -ForegroundColor Cyan
Set-Location $ProjectRoot

$buildOutput = cargo build --release 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "✗ Build failed" -ForegroundColor Red
    Write-Host $buildOutput -ForegroundColor Red
    exit 1
}

Write-Host "✓ Build complete" -ForegroundColor Green
Write-Host ""

# Step 3: Start FEAGI
Write-Host "[Step 3/3] Starting FEAGI server..." -ForegroundColor Cyan
Write-Host ""

# Build command arguments
$cargoArgs = @("run", "--release", "--", "--config", ".\feagi_configuration.toml")

# Add debug flag if requested
if ($DebugAll) {
    $cargoArgs += "--debug-all"
}

# Add genome if provided
if ($Genome -ne "") {
    if (-not (Test-Path $Genome)) {
        Write-Host "✗ Genome file not found: $Genome" -ForegroundColor Red
        exit 1
    }
    $cargoArgs += @("--genome", $Genome)
}

# Add any extra arguments
$cargoArgs += $ExtraArgs

Write-Host "Starting FEAGI with command:" -ForegroundColor Green
Write-Host "  cargo $($cargoArgs -join ' ')" -ForegroundColor Cyan
Write-Host ""
Write-Host "Press Ctrl+C to stop FEAGI" -ForegroundColor Yellow
Write-Host ""

# Execute
& cargo $cargoArgs

