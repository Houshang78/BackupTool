# Build the Windows variant with PyInstaller (run on WINDOWS).
#   PowerShell:   powershell -ExecutionPolicy Bypass -File packaging\build-windows.ps1
# Result: dist\backuptool.exe  (a single portable file, runnable from the drive too)
#
# Prerequisite: Python 3 (python.org). This script then installs pyinstaller + PySide6.

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)   # project root

Write-Host "==> Installing dependencies ..."
python -m pip install --upgrade pip
python -m pip install pyinstaller PySide6

Write-Host "==> PyInstaller building backuptool.exe ..."
# --onefile: a single portable .exe ; --console: CLI works, GUI starts with no arguments.
# PyInstaller ships its own PySide6 hooks (Qt plugins are bundled).
pyinstaller --noconfirm --onefile --console --name backuptool `
  --collect-submodules PySide6 `
  --add-data "backuptool/lang;backuptool/lang" `
  run.py

Write-Host ""
Write-Host "Done: dist\backuptool.exe"
Write-Host "  GUI:  double-click dist\backuptool.exe"
Write-Host "  CLI:  dist\backuptool.exe backup C:\Users\You\Documents -d E:\Backup -s my-pc --progress"
Write-Host ""
Write-Host "Build the installer (optional, needs Inno Setup):"
Write-Host "  ISCC packaging\win\backuptool.iss"
