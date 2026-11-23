@echo off
rem Opens the TTED cube terminal (cube> prompt). Double-click it, or let cube_autostart.pyw open it on plug-in.
title TTED Cube
python "%~dp0cube_control.py" %*
rem Keep the window open if something went wrong (e.g. the port is busy), so the message can be read.
if errorlevel 1 pause
