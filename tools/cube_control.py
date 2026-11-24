"""TTED LED cube remote: type a keyword, the cube switches to that light show.

Runs on the PC (not on the Pico), talks to the Rust firmware over USB serial.

    python tools/cube_control.py              interactive: type plasma, fire, lava, list, speed, auto, off, quit ...
    python tools/cube_control.py fire         one-shot: switch to "fire" and exit
    python tools/cube_control.py --port COM14 pick the port by hand (normally found automatically)

Needs pyserial:  pip install pyserial
Close any other program that has the cube's port open (VS Code serial monitor, MicroPico) first.
"""

import argparse
import os
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
CUBE_PID = 0x000A  # the Rust cube firmware
CUBE_SERIAL = "TTED-CUBE"

# Words handled here and sent as the firmware's one-letter commands (everything else goes as "@word").
LETTER_COMMANDS = {
    "measure": "m", "stats": "m",
    "test": "t", "selftest": "t",
    "faster": "+", "slower": "-",          # 595 clock one step
    "overclock": ">", "underclock": "<",  # CPU/PIO clock one step (unlimited build)
    "info": "h",
}

# The presets, grouped for the 'list' chart. Names must match the firmware's (its @list gives the numbers);
# a preset the firmware has but this table doesn't goes under "Other".
CATEGORIES = [
    ("Ambient & glow", [
        ("plasma", "Soft glowing blobs drifting through the cube"),
        ("lava lamp", "Three big blobs drifting and melting together"),
        ("heartbeat", "Lub-dub pulse, brightest in the centre"),
        ("sphere pulse", "A glowing ball growing and shrinking"),
        ("shells", "Cube-shaped shells pulsing out from the centre"),
        ("twinkle", "Random stars flashing on and fading out"),
    ]),
    ("Waves & sweeps", [
        ("diagonal wave", "Thin bright bands rolling corner to corner"),
        ("ripple", "Rings spreading over a water surface"),
        ("plane sweep", "A wall of light sweeping along each axis"),
        ("spinner", "A sheet of light spinning like a revolving door"),
    ]),
    ("Spirals & orbits", [
        ("double helix", "Two strands twisting as they climb (DNA)"),
        ("tornado", "A spinning funnel, narrow at the bottom"),
        ("corkscrew", "Two comets chasing around the outside, climbing"),
        ("orbits", "Three planets on tilted orbits around a sun"),
    ]),
    ("Particles & physics", [
        ("bouncing balls", "Three balls bouncing with gravity"),
        ("fountain", "Sparks spraying up from the middle, falling back"),
        ("fireworks", "A rocket climbs and bursts into sparks"),
        ("warp", "Stars rushing from back to front"),
        ("snake", "A dot wandering through the cube with a tail"),
        ("fill", "Fills one random LED at a time, then empties"),
    ]),
    ("Weather & elements", [
        ("fire", "Flames rising and flickering from the bottom"),
        ("lightning", "Flickering cloud on top, bolts striking down"),
        ("rain", "Drops falling from the top with trails"),
    ]),
    ("Classics", [
        ("cubelib demo", "The original CubeLibDemo.py sequences"),
        ("quadrum tted.txt", "The Quadrum animation file"),
    ]),
]

HELP = """\
Presets
  <name>                  play that preset (repeats until 'auto'); part of a name or its number works too
  list                    all presets, grouped, with what they do (> = playing now)
  auto                    play all presets one after another
  next / prev             skip to the next / previous preset
  off / on                all LEDs dark / back on (to the preset that was playing)
Brightness (5 steps: 5 = 100%, 4 = 80%, 3 = 60%, 2 = 40%, 1 = 20%)
  bright / dim            one step brighter / dimmer
  high / low              straight to 5 (full) / 1 (dimmest)
  brightness <1-5>        set it, e.g. 'brightness 3'; 'brightness' alone shows it
Buttons on the cube
  1 (GP13) next preset    2 (GP10) previous preset    3 (GP11) on/off
  4 (GP12) brightness one step down: 5 -> 4 -> 3 -> 2 -> 1 -> back to 5
Speed (all change live, no restart)
  speed                   show the current speed (595 clock, CPU clock, refresh rate, tempo)
  speed <MHz>             set the 595 clock, e.g. 'speed 80' (the default), 'speed 84' (fastest clean), 'speed 21'
  jason                   Jason's setting: the original design speed, 595 clock 20 MHz, CPU 240 MHz
  faster / slower         595 clock one step up / down
  overclock / underclock  CPU clock one step up / down (fast 'unlimited' build only)
  tempo <percent>         how fast the animations play: 'tempo 200' = twice as fast, 'tempo 50' = half
  tempo                   show the tempo
  reset                   back to defaults: power-up speed (80 MHz), tempo 100%, full brightness, presets cycling
Checks
  measure                 full refresh report
  test                    self-test (checks the pins at 595 clocks up to CPU/4)
  info                    the firmware's own help text
  probe all               test pattern: every column off (also the unused bits 2 and 18); anything lit = hardware
  walk                    light every LED one at a time (1 s each) and print its number and position
  walk <n>                light only LED n (0-124) and hold; next / prev (or buttons 1 / 2) step one LED
  walk stop               hold the LED the walk is on
  probe dark            test pattern: every layer AND column off; anything lit = powered by the board itself
  probe <bit>           test pattern: only that 595 bit (0-31) on; says which column / chip / pin it is
                          ('reset' leaves the probe)
Other
  help                    this list
  quit                    close the terminal (the cube keeps playing)"""


def find_port():
    ports = list(serial.tools.list_ports.comports())
    for p in ports:
        # the Rust cube firmware: PID 000A (Windows often reports no serial number)
        if p.vid == PICO_VID and (p.serial_number == CUBE_SERIAL or p.pid == CUBE_PID):
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
                # unplugged: end the program, which also closes the window cube_autostart opened
                print("\n[cube disconnected]")
                self.running = False
                os._exit(0)
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

    def ask(self, keyword, wait=0.5):
        """Send '@keyword' without echoing the answer; return the lines the cube printed."""
        start = len(self.lines)
        self.echo = False
        self.ser.write(f"@{keyword}\r".encode())
        time.sleep(wait)
        self.echo = True
        return self.lines[start:]

    def presets(self):
        """[(number, name, playing_now)], 'auto' or 'repeating', and the speed line, from the cube."""
        found, state = [], ""
        for line in self.ask("list"):
            m = re.match(r"^([ >])\s*(\d+)\s+(.+)$", line)
            if m:
                found.append((int(m.group(2)), m.group(3).strip(), m.group(1) == ">"))
            m = re.match(r"^now: .*\((\w+)\)", line)
            if m:
                state = m.group(1)
        speed = next((l for l in self.ask("speed", 0.3) if l.startswith("speed:")), "")
        return found, state, speed

    def close(self):
        self.running = False
        time.sleep(0.15)
        self.ser.close()


def show_list(cube):
    found, state, speed = cube.presets()
    if not found:
        print("  (no answer from the cube)")
        return
    by_name = {name.lower(): (num, name, now) for num, name, now in found}
    playing = next((name for _, name, now in found if now), "?")
    width = 74
    print()
    print("=" * width)
    print(f" Now playing: {playing}  ({state or '?'})")
    if speed:
        print(" " + speed.replace("speed: ", "Speed: "))
    print("=" * width)
    listed = set()
    groups = [(title, items) for title, items in CATEGORIES]
    others = [(n.lower(), "") for _, n, _ in found if n.lower() not in {k for _, it in CATEGORIES for k, _ in it}]
    if others:
        groups.append(("Other", others))
    for title, items in groups:
        rows = [(by_name[k], desc) for k, desc in items if k in by_name]
        if not rows:
            continue
        print(f"\n {title.upper()}")
        print(f" {'-' * (width - 2)}")
        for (num, name, now), desc in rows:
            mark = ">" if now else " "
            print(f" {mark}{num:3}  {name:<17} {desc}")
            listed.add(num)
    print(f"\n {'-' * (width - 2)}")
    print(" Type a name or number to play it. 'auto' = cycle all, 'off'/'on', 'dim'/'bright', 'help' = all commands.")
    print(" Brightness 1-5: 'bright', 'dim', 'brightness 3'. Buttons: 1 next, 2 previous, 3 on/off, 4 brightness.")
    print(" Speed: 'speed 80', 'jason' (original 20 MHz), 'tempo 150' (animation pace), 'reset' (defaults).")
    print("=" * width)
    print()


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


def run(cube, word):
    """One command from the prompt (or the command line)."""
    w = word.strip().lower()
    if w in ("list", "presets", "ls"):
        show_list(cube)
    elif w in ("help", "?"):
        print(HELP)
    else:
        cube.send(word)
        # the cube's answer prints before the next prompt (measure / test / clock changes take longer)
        key = w.replace(" ", "")
        slow = (LETTER_COMMANDS.get(key) in ("m", "t", ">", "<") or (key.startswith("speed") and key != "speed")
                or key in ("jason", "reset", "default", "defaults"))
        time.sleep(3.0 if slow else 0.3)


def main():
    ap = argparse.ArgumentParser(description="Switch the TTED LED cube's light show by keyword.")
    ap.add_argument("words", nargs="*", help="command to send, then exit (leave out for interactive mode)")
    ap.add_argument("--port", help="serial port, e.g. COM14 (default: find the cube automatically)")
    args = ap.parse_args()

    port = args.port or find_port()
    if not port:
        sys.exit("cube not found: is it plugged in (and not in BOOTSEL mode)? Or give --port COMx")
    cube = Cube(port)

    if args.words:
        run(cube, " ".join(args.words))
        time.sleep(0.2)
        cube.close()
        return

    print(f"TTED cube connected on {port}")
    show_list(cube)
    names = [name for _, name, _ in cube.presets()[0]]
    setup_completion(names + ["list", "auto", "next", "prev", "off", "on", "bright", "dim", "high", "low", "brightness", "help", "quit", "speed", "tempo", "jason", "reset"]
                     + list(LETTER_COMMANDS))

    try:
        while cube.running:
            try:
                word = input("cube> ").strip()
            except EOFError:
                break
            if word.lower() in ("quit", "exit", "q"):
                break
            if word:
                run(cube, word)
    except KeyboardInterrupt:
        print()
    cube.close()


if __name__ == "__main__":
    main()
