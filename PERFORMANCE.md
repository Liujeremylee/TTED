# LED Cube refresh performance

All numbers measured on the TTED 5×5×5 cube's Raspberry Pi Pico 2 (RP2350), Nov. 2026, with the Rust firmware's
`m` command (`dist/cube-rs.uf2`, normal build).

## Current firmware (Rust, normal build)

**Clocks**

| | |
|---|---|
| CPU | 240 MHz (stock 150 MHz) |
| PIO | 40 MHz (CPU ÷ 6) |
| 595 shift clock | 20 MHz (the board's design speed) |
| Flash | 80 MHz |

**LED refresh** (done entirely by the PIO and DMA)

| | Measured |
|---|---|
| Time per word on the pins | 1,649 ns (66 PIO cycles: 64 for the bits + 2 for the latch) |
| Words per second | 606,050 |
| Each layer refreshed | 121,210 times/s |
| Full brightness cycles (5 layers × 10 levels) | **12,121 per second** |
| CPU instructions per layer refresh | **0** |

**CPU cost per new picture** (only when the animation changes)

| | |
|---|---|
| Table build | 673–785 CPU cycles (2.8–3.3 µs), 4 instructions per LED |
| `show()` total | 907 CPU cycles (3.8 µs), no waiting for the DMA |
| New picture on the LEDs | within one pass (at most about 83 µs) |

**Checks:** boot self-test PASS; PIO1 pin capture equals the table bit for bit (150 of 150 words).

## Progress

| | Original MicroPython | MicroPython + PIO/DMA | Rust (now) |
|---|---|---|---|
| Full brightness cycles/s | 60 | 11,111 | **12,121** |
| CPU time per layer refresh | 156 µs | 0 | 0 |
| CPU time per new picture | — | about 28 µs, 250×/s | **3.8 µs, only on change** |
| PIO cycles per word | — | 72 | **66** |

## `unlimited` build (`dist-unlimited/`, 595 rating ignored)

| CPU = PIO clock | 595 clock | Full brightness cycles/s | Status |
|---|---|---|---|
| 240 MHz | 120 MHz | 72,726 | Pico side verified; 595s not rated this fast |
| 264 MHz | 132 MHz | 79,999 | Pico side verified (pin self-test, 2,000 checked builds) |
| 288–400 MHz | 144–200 MHz | — | not tested |

The limit at 20 MHz is the 595 shift clock. See the alternatives to the 595 (parallel data lines, 74AHC/74LVC595,
drivers with built-in PWM) for ways past it.
