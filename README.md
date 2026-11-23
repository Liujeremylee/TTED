# TTED

BCIT TTED 5×5×5 LED cube on a Raspberry Pi Pico 2. The Rust firmware (`rust/`, ready-to-flash builds in `dist/` and
`dist-unlimited/`) refreshes the LEDs with PIO + DMA and plays the light shows below. See `rust/README.md` for
building and flashing, and `PERFORMANCE.md` for the measured speeds. The `.py` files are the original MicroPython
version.

## Controlling the cube

### The cube terminal (opens by itself when the cube is plugged in)
One-time setup: `pip install pyserial`, then (from the repo root)
`powershell -ExecutionPolicy Bypass -File tools\install_autostart.ps1`.
From then on, plugging the cube in opens a **TTED Cube** terminal with a `cube>` prompt; unplugging it closes the
terminal. Without the setup: double-click `tools\cube_terminal.cmd`, or run `python tools\cube_control.py`.
Uninstall: `install_autostart.ps1 -Remove`.

Only one program can use the cube's port at a time: close the terminal before using another serial monitor,
MicroPico, or `bundle.ps1`/picotool flashing via `b`.

### All commands
Type them at the `cube>` prompt and press Enter. In any other serial terminal (any baud rate) use the
**Serial** column: keywords start with `@` and end with Enter; one-letter commands act at once, no Enter.

| `cube>` prompt | Serial | What it does |
|---|---|---|
| *a preset name*, e.g. `fire` | `@fire` | Play that preset; it repeats until `auto`. Part of a name works (`lava`, `helix`, `bounce`), and so does its number from `list` |
| `list` | `@list` | Every preset, grouped by kind (ambient, waves, spirals, particles, weather, classics) with a one-line description; `>` marks the one playing, and the current speed is shown on top |
| `auto` | `@auto` | Play all presets one after another (the start-up behaviour) |
| `next` | `@next` or `n` | Skip to the next preset (button 1 on GP13 does the same) |
| `off` | `@off` | All LEDs dark (`auto` or any preset name turns them back on) |
| `measure` | `m` | Refresh report: clocks, measured refresh rate, table build times, self-test result |
| `test` | `t` | Self-test: table vs the original CubeLib algorithm, and the words on the pins (595 clock ≤ CPU/4) |
| `speed` | `@speed` | Show the speed: 595 clock, CPU clock, PIO divider, full brightness cycles/s |
| `speed 84` | `@speed 84` | Set the 595 clock to the fastest setting at or below that many MHz (the fast build also picks the CPU clock). 84 = fastest clean on this cube, 21 ≈ the board's design speed. Back to 84 MHz at every power-up |
| `faster` / `slower` | `+` / `-` | 595 clock one step up / down (CPU ÷ 1, 2, 3 ...) |
| `overclock` / `underclock` | `>` / `<` | CPU = PIO clock one step up / down, 240–400 MHz (`unlimited` build only) |
| `info` | `h` | Firmware help text |
| `help` | — | This list, in the terminal |
| `quit` | — | Close the terminal (the cube keeps playing) |
| — | `b` | Reboot into BOOTSEL (flash mode). Not available at the `cube>` prompt on purpose |

One-shot from a script: `python tools\cube_control.py fire` (or `auto`, `off`, `measure` ...) sends one command
and exits.

### Presets

| # | Name | What it looks like |
|---|---|---|
| 0 | plasma | Soft glowing blobs drifting through the cube |
| 1 | twinkle | Random stars flash on and fade out |
| 2 | plane sweep | A wall of light sweeps along x, then y, then up and down |
| 3 | shells | Cube-shaped shells pulse out from the centre |
| 4 | double helix | Two strands twisting as they climb, with dim rungs |
| 5 | orbits | Three planets on tilted orbits around a dim sun |
| 6 | diagonal wave | Thin bright bands rolling corner to corner |
| 7 | bouncing balls | Three balls bouncing with gravity, with trails |
| 8 | fire | Flames rising and flickering from the bottom |
| 9 | lightning | A flickering cloud on top; jagged bolts strike down |
| 10 | heartbeat | Lub-dub pulse, brightest in the centre |
| 11 | fountain | Sparks spray up from the middle and fall back |
| 12 | tornado | A spinning funnel, wide at the top |
| 13 | lava lamp | Three blobs drifting and melting together |
| 14 | warp | Stars rushing from back to front |
| 15 | fill | Fills one random LED at a time, then empties |
| 16 | rain | Drops falling with trails |
| 17 | ripple | Rings spreading over a water surface |
| 18 | spinner | A sheet of light spinning like a revolving door |
| 19 | fireworks | A rocket climbs and bursts into sparks |
| 20 | sphere pulse | A ball growing and shrinking like a heartbeat |
| 21 | corkscrew | Two comets chasing around the outside, climbing |
| 22 | snake | A dot wandering through the cube with a tail |
| 23 | CubeLib demo | The CubeLibDemo.py sequences (repeats) |
| 24 | Quadrum TTED.txt | The Quadrum animation file (repeats) |

0–15 are in `rust/src/lightshow.rs`, 16–22 in `rust/src/animations.rs`. With `auto`, 0–22 play in a loop; button 1 / `next`
continues past 22 to the demo and Quadrum.
