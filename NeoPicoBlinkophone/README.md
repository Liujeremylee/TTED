# NeoPicoBlinkophone

A music-reactive LED strand on a Raspberry Pi Pico. It listens to a stereo audio signal and drives WS2812 strands with one of seven light shows, chosen by three switches.

## Folders

| Folder | What's in it |
|---|---|
| `NeoPicoBlinkophoneV1/` | The sketch to upload normally. |
| `NeoPicoBlinkophoneV1_Tuning/` | The same sketch, plus a binary stream of every frame's audio over USB, for recording songs to tune against. Generated from the main sketch: don't edit it by hand. |
| `tools/` | `make_tuning.ps1` regenerates the tuning sketch; `capture.ps1` records the stream to a file; `Sim.cs` replays a recording through each preset offline. |
| `recordings/` | Songs recorded on the Pico with the tuning sketch (60 frames a second): hyper, balanced, calm and hyperish songs, and silence followed by music. `lowvol`, `hats_check` and `pico2_mood` came from a second Pico whose left input reads about 5 times quieter than its right. |
| `old-versions/` | Earlier sketches with presets that were later replaced (a level meter and a storm effect). |

## Hardware

| Pin | Connection |
|---|---|
| GP0 | Main LED strand (38 LEDs; LEDs 1-3 loop round next to LED 38) |
| GP1 | Second LED strand (36 LEDs), optional |
| A0 / GP26 | Left audio channel |
| A1 / GP27 | Right audio channel |
| A2 / GP28 | Knob: sensitivity by default, or brightness (`KNOB_CONTROLS`) |
| GP16, GP17, GP18 | Preset switches |

## Presets

The code reads the three switch pins as a 3-bit number (`SWITCH_PINS`). The names below are the switch labels on the build, where "16" is the lowest bit.

| Switches on | Preset |
|---|---|
| none | Off |
| 16 | Bass flash: the whole strand flashes blue to white on bass hits, white sparkles on hi-hats |
| 17 | Bars: blue (low), white (mid) and red (high) grow out from the middle |
| 17 + 16 | Stereo spectrum: left channel on one side of the middle, right on the other |
| 18 | Mood glow: slow colour that follows the tone of the music |
| 18 + 16 | Ripples: pulses out from the middle on bass hits, teal ripples on vocals, sparks on hi-hats |
| 18 + 17 | Starfield: twinkling stars over a deep purple sky that glows on bass hits |
| 18 + 17 + 16 | Fire: flames from both ends, taller with the bass |

## Building and uploading

Board: Raspberry Pi Pico, using Earle Philhower's arduino-pico core (tested with 6.3.2). Libraries: FastLED 3.10.9 and arduinoFFT 2.0.4.

```powershell
arduino-cli compile --fqbn rp2040:rp2040:rpipico NeoPicoBlinkophoneV1
arduino-cli upload  --fqbn rp2040:rp2040:rpipico -p COM4 NeoPicoBlinkophoneV1
```

It runs at 60 frames a second. Core 1 records audio non-stop, and core 0 analyses it and drives the LEDs. It uses about 103 KB of flash and 21 KB of RAM.

## Tuning workflow

1. After changing the main sketch, run `tools\make_tuning.ps1` to regenerate the tuning sketch.
2. Upload the tuning sketch, play a song, and run `tools\capture.ps1 -Seconds 30 -OutFile song.bin`.
3. Load `tools\Sim.cs` in PowerShell with `Add-Type` and run `[Sim]::Load("song.bin", "name").Report($p)` to see how each preset behaves (flashes per second, how many land on the beat, how much of the strand is lit). `$p` is a `P` object holding the settings to try; its defaults are older values, so set its fields to match the sketch first. Re-running `Add-Type` in the same PowerShell session needs the class names changed.
4. Put the normal sketch back on the Pico when finished.

Each recorded frame is a 522-byte packet: `0xA5 0x5A`, mode, brightness, frame number, wait time, work time, then 128 left and 128 right frequency levels, each stored as `round(log2(1 + level) * 3800)`.
