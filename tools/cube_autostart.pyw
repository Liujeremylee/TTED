"""Background watcher: opens the cube terminal (cube_terminal.cmd) whenever the TTED cube is plugged in.

Runs with pythonw (no window). install_autostart.ps1 puts it in the Windows Startup folder so it starts at login.
  * cube plugged in       -> a "TTED Cube" terminal opens with the cube> prompt
  * cube unplugged        -> that terminal closes by itself
  * terminal closed by you -> it opens again the next time the cube is plugged in (not before)
Only one watcher runs at a time.
"""

import ctypes
import subprocess
import sys
import time
from pathlib import Path

import serial.tools.list_ports

PICO_VID = 0x2E8A
CUBE_PID = 0x000A  # the Rust cube firmware (MicroPython is 0x0005; BOOTSEL mode has no serial port)
CUBE_SERIAL = "TTED-CUBE"  # Windows often reports no serial number, so the PID is what usually matches
HERE = Path(__file__).resolve().parent
TERMINAL = HERE / "cube_terminal.cmd"


def cube_present():
    return any(p.vid == PICO_VID and (p.pid == CUBE_PID or p.serial_number == CUBE_SERIAL)
               for p in serial.tools.list_ports.comports())


def main():
    # one watcher only: a second copy (e.g. started again by the installer) exits at once
    ctypes.windll.kernel32.CreateMutexW(None, False, "TTED_cube_autostart")
    if ctypes.windll.kernel32.GetLastError() == 183:  # ERROR_ALREADY_EXISTS
        sys.exit(0)

    was_present = False  # so a cube that is already plugged in at login gets its terminal too
    while True:
        try:
            present = cube_present()
        except Exception:
            present = False
        if present and not was_present:
            time.sleep(1.0)  # give the USB serial port a moment to come up
            subprocess.Popen(["cmd", "/c", str(TERMINAL)], cwd=str(HERE),
                             creationflags=subprocess.CREATE_NEW_CONSOLE)
        was_present = present
        time.sleep(1.5)


if __name__ == "__main__":
    main()
