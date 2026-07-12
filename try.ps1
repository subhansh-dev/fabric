# Fabric - Quick Test Script
# Usage: powershell -ExecutionPolicy Bypass -File try.ps1

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"

Write-Host ""
Write-Host "  FABRIC - a compiled language for robots" -ForegroundColor Cyan
Write-Host ""

Write-Host "Building compiler..." -ForegroundColor Yellow
& $cargo build --release 2>$null
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed. Is Rust installed?" -ForegroundColor Red
    exit 1
}

$exe = ".\target\release\fabric-lang.exe"

Write-Host ""
Write-Host "Checking drone.fab..." -ForegroundColor Yellow
& $exe check --file examples\drone.fab

Write-Host ""
Write-Host "Generating Python..." -ForegroundColor Yellow
& $exe build --target python --file examples\drone.fab --output examples\drone.py
Write-Host "  -> examples\drone.py" -ForegroundColor Green

Write-Host ""
Write-Host "Generating C..." -ForegroundColor Yellow
& $exe build --target c --file examples\drone.fab --output examples\drone.c
Write-Host "  -> examples\drone.c" -ForegroundColor Green

Write-Host ""
Write-Host "Timing analysis..." -ForegroundColor Yellow
& $exe timing --file examples\drone.fab

Write-Host ""
Write-Host "Running tests..." -ForegroundColor Yellow
& $cargo test --quiet 2>$null

Write-Host ""
Write-Host "Done!" -ForegroundColor Green
