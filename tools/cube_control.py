"""TTED LED cube remote: type a keyword, the cube switches to that light show.

Runs on the PC (not on the Pico), talks to the Rust firmware over USB serial.

    python tools/cube_control.py              interactive: type plasma, fire, lava, auto, off, list, quit ...
    python tools/cube_control.py fire         one-shot: switch to "fire" and exit
    python tools/cube_control.py --port COM14 pick the port by hand (normally found automatically)

Needs pyserial:  pip install pyserial
Close any other program that has the cube's port open (VS Code serial monitor, MicroPico) first.
"""

import argparse
import re
import sys
import threading
import time

try:
    import serial
    import serial.tools.list_ports
except ImportError:
    sys.exit("pyserial is missing: pip install pyserial")

PICO_VID = 0x2E8A
CUBE_SERIAL = "TTED-CUBE"

# Words handled here and sent as the firmware's one-letter commands (everything else goes as "@word").
LETTER_COMMANDS = {
    "measure": "m", "stats": "m",
    "test": "t", "selftest": "t",
    "faster": "+", "slower": "-",          # 595 clock
    "overclock": ">", "underclock": "<",  # CPU/PIO clock (unlimited build)
    "info": "h",
}

HELP = """\
Type a preset name (or part of one, or its number) and press Enter:  plasma, fire, lava, tornado, 3 ...
  list        all presets (the one playing is marked >)
  auto        go back to cycling through all of them
  next        skip to the next one
  off         all LEDs dark
  measure     refresh-rate report        test     self-test
  faster / slower         595 clock up / down a step
  overclock / underclock  CPU clock up / down a step (unlimited build)
  quit        leave (the cube keeps playing)"""


def find_port():
    ports = list(serial.tools.list_ports.comports())
    for p in ports:
        if p.vid == PICO_VID and p.serial_number == CUBE_SERIAL:
            return p.device
    for p in ports:
        if p.vid == PICO_VID:
            return p.device
    return None


class Cube:
    def __init__(self, port):
        try:
            self.ser = serial.Serial(port, 115200, timeout=0.1)
        except serial.SerialException as e:
            sys.exit(f"can't open {port}: {e}\n(is another program - VS Code serial monitor, MicroPico - using it?)")
        self.lines = []          # everything the cube printed, line by line
        self.echo = True         # print the cube's output as it arrives
        self.running = True
        self._buf = ""
        threading.Thread(target=self._reader, daemon=True).start()

    def _reader(self):
        while self.running:
            try:
                data = self.ser.read(256)
            except serial.SerialException:
                print("\n[cube disconnected]")
                self.running = False
                return
            if not data:
                continue
            self._buf += data.decode("utf-8", "replace")
            while "\n" in self._buf:
                line, self._buf = self._buf.split("\n", 1)
                line = line.rstrip("\r")
                self.lines.append(line)
                if self.echo and line.strip():
                    print(f"  cube: {line}")

    def send(self, word):
        word = word.strip()
        if not word:
            return
        key = word.lower().replace(" ", "")
        if key in LETTER_COMMANDS:
            self.ser.write(LETTER_COMMANDS[key].encode())
        else:
            self.ser.write(f"@{word}\r".encode())

    def preset_names(self):
        """Ask the cube for its list and return the preset names (for tab completion)."""
        start = len(self.lines)
        self.echo = False
        self.ser.write(b"@list\r")
        time.sleep(0.5)
        self.echo = True
        names = []
        for line in self.lines[start:]:
            m = re.match(r"^[ >]\s*\d+\s+(.+)$", line)
            if m:
                names.append(m.group(1).strip())
        return names

    def close(self):
        self.running = False
        time.sleep(0.15)
        self.ser.close()


def setup_completion(words):
    try:
        import readline  # on Windows: pip install pyreadline3 (optional, for Tab completion)
    except ImportError:
        return
    words = sorted(set(w.lower() for w in words))

    def complete(text, state):
        hits = [w for w in words if w.startswith(text.lower())]
        return hits[state] if state < len(hits) else None

    readline.set_completer_delims("")
    readline.set_completer(complete)
    readline.parse_and_bind("tab: complete")


def main():
    ap = argparse.ArgumentParser(description="Switch the TTED LED cube's light show by keyword.")
    ap.add_argument("words", nargs="*", help="keyword(s) to send, then exit (leave out for interactive mode)")
    ap.add_argument("--port", help="serial port, e.g. COM14 (default: find the cube automatically)")
    args = ap.parse_args()

    port = args.port or find_port()
    if not port:
        sys.exit("cube not found: is it plugged in (and not in BOOTSEL mode)? Or give --port COMx")
    cube = Cube(port)

    if args.words:
        word = " ".join(args.words)
        cube.send(word)
        # let the cube's answer print (measure / test / clock steps take a couple of seconds)
        slow = LETTER_COMMANDS.get(word.lower().replace(" ", "")) in ("m", "t", ">", "<")
        time.sleep(3.0 if slow else 0.4)
        cube.close()
        return

    print(f"connected to the cube on {port}")
    names = cube.preset_names()
    if names:
        print("presets: " + ", ".join(names))
    print('type "help" for commands, "quit" to leave\n')
    setup_completion(names + ["list", "auto", "next", "off", "help", "quit"] + list(LETTER_COMMANDS))

    try:
        while cube.running:
            try:
                word = input("cube> ").strip()
            except EOFError:
                break
            if word.lower() in ("quit", "exit", "q"):
                break
            if word.lower() in ("help", "?"):
                print(HELP)
                continue
            cube.send(word)
            time.sleep(0.25)  # the cube's reply prints before the next prompt
    except KeyboardInterrupt:
        print()
    cube.close()


if __name__ == "__main__":
    main()
