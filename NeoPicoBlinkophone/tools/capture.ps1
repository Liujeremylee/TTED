param([int]$Seconds = 10, [string]$OutFile)

# Record the Pico's binary tuning stream to a file, then report how many good frames arrived.
$port = ((Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match 'VID_2E8A' -and $_.FriendlyName -match '(COM\d+)' }).FriendlyName -replace '.*\((COM\d+)\).*', '$1')
if (-not $port) { throw "Pico serial port not found" }
$sp = New-Object System.IO.Ports.SerialPort $port, 115200
$sp.DtrEnable = $true
$sp.ReadBufferSize = 4MB
$sp.Open()
$fs = [System.IO.File]::Create($OutFile)
$buf = New-Object byte[] 65536
$end = (Get-Date).AddSeconds($Seconds)
try {
  while ((Get-Date) -lt $end) {
    $n = $sp.BytesToRead
    if ($n -gt 0) { $r = $sp.Read($buf, 0, [math]::Min($n, $buf.Length)); $fs.Write($buf, 0, $r) }
    else { Start-Sleep -Milliseconds 5 }
  }
} finally { $fs.Close(); $sp.Close() }

# Quick check: find packets (0xA5 0x5A + 522 bytes), count them and look for dropped frames
$b = [System.IO.File]::ReadAllBytes($OutFile)
$P = 522; $i = 0; $frames = 0; $dropped = 0; $prev = -1; $waitSum = 0; $workSum = 0; $modes = @{}
while ($i + $P -le $b.Length) {
  if ($b[$i] -eq 0xA5 -and $b[$i + 1] -eq 0x5A) {
    $fno = $b[$i + 4] + 256 * $b[$i + 5]
    if ($prev -ge 0) { $gap = ($fno - $prev + 65536) % 65536; if ($gap -gt 1) { $dropped += $gap - 1 } }
    $prev = $fno; $frames++
    $waitSum += $b[$i + 6] + 256 * $b[$i + 7]; $workSum += $b[$i + 8] + 256 * $b[$i + 9]
    $modes[[int]$b[$i + 2]] = 1 + [int]$modes[[int]$b[$i + 2]]
    $i += $P
  } else { $i++ }
}
"port $port | {0:N0} bytes | {1} frames ({2:N1}/s) | dropped {3} | avg wait {4:N0}us, work {5:N0}us | modes seen: {6}" -f $b.Length, $frames, ($frames / $Seconds), $dropped, ($waitSum / [math]::Max(1, $frames)), ($workSum / [math]::Max(1, $frames)), (($modes.Keys | Sort-Object | ForEach-Object { "$_ ($($modes[$_]))" }) -join ', ')
