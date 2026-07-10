@echo off
rem OddSports daily generation pass — scheduled at 09:00 local.
rem Grades every past slate first (zero AI cost), then generates today's.
cd /d X:\OddSports
if not exist logs mkdir logs
echo [%date% %time%] daily run start >> logs\pipeline.log
target\release\oddsports-pipeline.exe >> logs\pipeline.log 2>&1
echo [%date% %time%] daily run exit %errorlevel% >> logs\pipeline.log
