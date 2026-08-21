@echo off
rem Neat launcher for the agpeer dev stack (frontend + core + logs).
rem Usage:
rem   dev                full stack (opens the browser at http://127.0.0.1:5173)
rem   dev -NoUi          core only (no frontend window)
rem   dev -Trace         heavier trace logging
rem   dev -Creds         prompt for Soulseek credentials first
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0run\dev.ps1" %*
