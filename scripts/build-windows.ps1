# Build ALL Windows artifacts in one go: Rust CLI+GUI .exe and the Python .exe.
# Run on Windows:  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
# Output: dist\windows\
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$out  = Join-Path $root "dist\windows"
New-Item -ItemType Directory -Force $out | Out-Null

Write-Host "==> Rust (CLI + GUI, release)"
Push-Location (Join-Path $root "rust")
cargo build --release --bin backuptool
cargo build --release --features gui --bin backuptool-gui
Copy-Item target\release\backuptool.exe, target\release\backuptool-gui.exe $out -Force
Pop-Location

Write-Host "==> Python (.exe via PyInstaller)"
Push-Location (Join-Path $root "python")
powershell -ExecutionPolicy Bypass -File packaging\build-windows.ps1
Copy-Item dist\backuptool.exe (Join-Path $out "backuptool-python.exe") -Force
Pop-Location

Write-Host ""
Write-Host "Artifacts in $out :"
Get-ChildItem $out | Format-Table Name, Length
