# TTED LED Cube — Rust firmware (Raspberry Pi Pico 2 / RP2350)

Native firmware for the BCIT TTED 5×5×5 LED cube. It replaces MicroPython on the Pico. The Python files in the
repo root still work if you flash MicroPython back.

## How the refresh works
- **PIO0** shifts each 32‑bit word into the 595 chain (MSB first) and pulses the latch: **66 PIO cycles per word**.
  The next word is pulled during the latch pulse, so the pull costs no extra cycle.
- **DMA** channel 0 streams a 50‑word table (10 brightness slots × 5 layers) into the PIO. Channel 1 restarts it
  at the end of every pass. **The CPU executes zero instructions per layer.**
- `Cube::show()` builds a new table (about 675–785 CPU cycles, ≈3 µs, run from SRAM) into a third, free buffer and
  queues it. The DMA switches between passes, so a half-built picture is never shown and `show()` never waits.

Measured on the board (CPU 240 MHz, 595 clock 20 MHz): **12,120 full brightness cycles/s**, every layer
refreshed **121,200 times/s**, one word on the pins every 1650 ns.

## Build, flash, bundle
Requirements: Rust (`thumbv8m.main-none-eabihf` target) and `picotool`.

| Command (in this folder) | What it does |
|---|---|
| `cargo build --release` | Optimized build (opt-level 3, fat LTO, 1 codegen unit) |
| `cargo run --release` | Build and flash with picotool (the board must be in BOOTSEL mode) |
| `powershell -ExecutionPolicy Bypass -File .\bundle.ps1` | Optimized build + `../dist/cube-rs.uf2`, `cube-rs.elf`, `SHA256SUMS.txt` |

To flash the UF2 by hand: hold BOOTSEL while plugging the Pico in (or send `b` over USB serial), then copy
`dist/cube-rs.uf2` onto the drive that appears.

## USB serial commands (any terminal, any baud rate)
`m` measure & report · `t` self-test · `n` next animation · `+`/`-` faster/slower 595 clock · `b` reboot to BOOTSEL · `h` help

Button 1 (GP13) also switches the animation (CubeLib demo ↔ Quadrum `TTED.txt`).

The self-test uses PIO1 as a logic analyzer on GP1–GP3. It checks that the words reaching the 595s match the table
bit for bit, and that the table matches the original CubeLib algorithm.

## Settings (top of `src/main.rs`)
- `MAX_SCK_HZ` — 595 clock (20 MHz, as in the original SPI code).
- `HOLD` — extra PIO cycles per slot. Raise it if you see ghosting between layers (tested: `HOLD = 10` → 78 cycles).

## `unlimited` feature: ignore the 595 clock
`cargo build --release --features unlimited`, or `bundle.ps1 -Unlimited` → `../dist-unlimited/cube-rs.uf2`.

The PIO runs at the full CPU clock (divider 1, the fastest possible), so the 595 clock is **120 MHz**. That's
about 6× what a 74HC595 is rated for at 3.3 V. The latch/clock/data pads are set to fast slew and 12 mA.
Measured: **72,726 full brightness cycles/s**, every layer refreshed **727,260 times/s**, 275 ns per word.
The Pico's output is exact, but the 595s probably can't keep up, so expect wrong LEDs. `-` steps the clock
down while it runs (60, 40, 30 MHz ...). The pin-capture self-test only works at 30 MHz (CPU/4) and slower;
faster than that it reports SKIPPED.

## Windows build notes
- The host build tools need `dlltool` from Rust's GNU toolchain `self-contained` folder on PATH; `bundle.ps1` adds it.
- `Cargo.lock` pins `winapi-util 0.1.9`, `parking_lot 0.12.3`, `parking_lot_core 0.9.10` and `term 1.1.0`. These are
  build-time-only crates of the PIO assembler. The pins avoid newer versions that need a full MinGW assembler.
