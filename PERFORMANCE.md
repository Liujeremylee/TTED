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

Measured Nov. 23, 2026. The Pico-side checks (pin self-test at CPU/4, 2,000 table builds checked against the
reference) passed at every CPU clock from 240 to 400 MHz. What the 595s make of it, judged by eye on the cube
(with the stacked 595s; the result was the same before stacking):

| CPU | 595 clock | Full brightness cycles/s | On the cube |
|---|---|---|---|
| 400 MHz (÷1) | 200 MHz | 101,006 | one LED lit |
| 400 MHz (÷2) | 100 MHz | 50,505 | dark |
| 360 MHz (÷2) | 90 MHz | 45,455 | faint flicker |
| 342 MHz (÷2) | 85.5 MHz | 43,183 | past the edge |
| 336 MHz (÷2) | 84 MHz | 42,423 | clean (fastest clean; was the boot default) |
| 328 MHz (÷2) | 82 MHz | 41,414 | clean |
| **320 MHz (÷2)** | **80 MHz** | **40,403** | **clean — the boot default** |
| 320 MHz (÷8) | 20 MHz | 10,101 | clean |

Numbers are with the anti-ghosting table (each layer's 10 slots back to back, 2 blank words before each layer:
60 words per pass). Before it, every word switched layers and off LEDs glowed (low contrast) even at 20 MHz.

The limit at 20 MHz is the 595 shift clock. See the alternatives to the 595 (parallel data lines, 74AHC/74LVC595,
drivers with built-in PWM) for ways past it.
