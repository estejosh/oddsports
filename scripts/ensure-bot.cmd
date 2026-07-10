@echo off
rem OddSports bot watchdog — scheduled at logon + every 30 minutes.
rem Starts the bot only if it is not already running (two long-pollers
rem on one token fight over getUpdates).
cd /d X:\OddSports
if not exist logs mkdir logs
tasklist /FI "IMAGENAME eq oddsports-bot.exe" | find /I "oddsports-bot.exe" >nul
if errorlevel 1 (
  echo [%date% %time%] bot not running — starting >> logs\bot-watchdog.log
  start "" /b cmd /c "target\release\oddsports-bot.exe >> logs\bot.log 2>&1"
)
