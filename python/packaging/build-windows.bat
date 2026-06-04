@echo off
REM Build the Windows variant with PyInstaller (alternative to build-windows.ps1).
REM Run from the Command Prompt:  packaging\build-windows.bat
setlocal
cd /d "%~dp0\.."

echo ==> Installing dependencies ...
python -m pip install --upgrade pip || goto :err
python -m pip install pyinstaller PySide6 || goto :err

echo ==> PyInstaller building backuptool.exe ...
pyinstaller --noconfirm --onefile --console --name backuptool --icon packaging\win\backuptool.ico --collect-submodules PySide6 --add-data "backuptool/lang;backuptool/lang" run.py || goto :err

echo.
echo Done: dist\backuptool.exe
echo   GUI:  double-click dist\backuptool.exe
echo   CLI:  dist\backuptool.exe backup C:\Users\You\Documents -d E:\Backup -s my-pc --progress
goto :eof

:err
echo BUILD FAILED.
exit /b 1
