# Rebuild the tuning sketch from the main sketch: same code, plus the binary data stream for offline tuning.
$root = Split-Path $PSScriptRoot -Parent
$main = Join-Path $root "NeoPicoBlinkophoneV1\NeoPicoBlinkophoneV1.ino"
$tun  = Join-Path $root "NeoPicoBlinkophoneV1_Tuning\NeoPicoBlinkophoneV1_Tuning.ino"
$raw = [IO.File]::ReadAllText($main); $crlf = $raw.Contains("`r`n"); $c = $raw -replace "`r`n", "`n"

$swaps = @(
  @("#define DEBUG_TIMING 1  // set to 1 and open the Serial Monitor to see the mode, timing, levels and knob once a second",
    "// TUNING BUILD: streams every frame's audio data over USB in binary (not readable in the Serial Monitor).`n// Packet: 0xA5 0x5A, mode, brightness, frame number (2 bytes), wait us (2), work us (2),`n//         then 128 left and 128 right bucket loudnesses, each 2 bytes, encoded as round(log2(1 + loudness) * 3800)`n#define DEBUG_STREAM 1`n#define DEBUG_TIMING 0  // set to 1 and open the Serial Monitor to see the mode, timing, levels and knob once a second"),
  @("  readAudio();   // wait for core 1 to finish recording the next frame`n",
    "#if DEBUG_STREAM`n  uint32_t s0 = micros();`n#endif`n  readAudio();   // wait for core 1 to finish recording the next frame`n#if DEBUG_STREAM`n  uint32_t s1 = micros();`n#endif`n"),
  @("  FastLED.show();   // send the colours stored in leds[] and leds2[] out to the strands`n",
    "#if DEBUG_STREAM`n  streamFrame(s1 - s0, micros() - s1);`n#endif`n`n  FastLED.show();   // send the colours stored in leds[] and leds2[] out to the strands`n"),
  @("//----------------------------- CORE 1: records audio non-stop -----------------------------",
    @'
#if DEBUG_STREAM
// Send this frame's left and right spectrum, the mode and the timing to the computer, for offline tuning
void streamFrame(uint32_t waitUs, uint32_t workUs) {
  static uint8_t packet[10 + 2 * 2 * 128];
  static uint16_t frameNo = 0;
  auto put16 = [](uint8_t *p, uint32_t v) { if (v > 65535) v = 65535; p[0] = v & 0xFF; p[1] = v >> 8; };
  auto encode = [](float v) -> uint32_t { return (uint32_t)(log2f(1.0f + v) * 3800.0f + 0.5f); };

  packet[0] = 0xA5; packet[1] = 0x5A;
  packet[2] = routine;
  packet[3] = FastLED.getBrightness();
  put16(packet + 4, frameNo++);
  put16(packet + 6, waitUs);
  put16(packet + 8, workUs);
  for (int k = 0; k < 128; k++) {
    put16(packet + 10 + 2 * k,       encode(leftMag[k]));
    put16(packet + 10 + 256 + 2 * k, encode(rightMag[k]));
  }
  Serial.write(packet, sizeof(packet));
}
#endif

//----------------------------- CORE 1: records audio non-stop -----------------------------
'@ -replace "`r`n", "`n")
)
foreach ($s in $swaps) {
  if (-not $c.Contains($s[0])) { throw "Not found: $($s[0].Substring(0, [math]::Min(60, $s[0].Length)))" }
  $c = $c.Replace($s[0], $s[1])
}
if ($crlf) { $c = $c -replace "`n", "`r`n" }
[IO.File]::WriteAllText($tun, $c, (New-Object System.Text.UTF8Encoding($false)))
"tuning sketch rebuilt from main"
