@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-exe.ps1" %*
set RESULT=%ERRORLEVEL%
echo.
echo Exit-Code: %RESULT%. Eine Taste druecken zum Schliessen.
pause >nul
exit /b %RESULT%
