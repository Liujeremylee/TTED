# Build the optimized release firmware and package it in dist/:
#   cube-rs.uf2  - drag onto the RP2350 drive (hold BOOTSEL while plugging in, or send 'b' over USB serial)
#   cube-rs.elf  - for picotool / debuggers (debug symbols stay in the .elf; they never go to flash)
#   SHA256SUMS.txt
# Usage (from the rust folder):  powershell -ExecutionPolicy Bypass -File .\bundle.ps1
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# Tools: cargo, and the self-contained MinGW tools Rust's GNU host toolchain needs for build scripts.
$selfContained = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained"
$env:Path = "$env:USERPROFILE\.cargo\bin;$selfContained;$env:Path"
$picotool = (Get-Command picotool -ErrorAction SilentlyContinue).Source
if (-not $picotool) {
    $picotool = Get-ChildItem "$env:USERPROFILE\.pico-sdk\picotool" -Recurse -Filter picotool.exe -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $picotool) { throw "picotool not found (install it into ~/.pico-sdk/picotool or put it on PATH)" }

# Optimized build: opt-level 3, fat LTO, 1 codegen unit (see [profile.release] in Cargo.toml).
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$elf = "target\thumbv8m.main-none-eabihf\release\cube-rs"
$dist = Join-Path $PSScriptRoot "..\dist"
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item $elf (Join-Path $dist "cube-rs.elf") -Force
& $picotool uf2 convert -t elf $elf -t uf2 (Join-Path $dist "cube-rs.uf2") --family rp2350-arm-s
if ($LASTEXITCODE -ne 0) { throw "picotool uf2 convert failed" }

$lines = foreach ($f in "cube-rs.uf2", "cube-rs.elf") {
    $h = (Get-FileHash (Join-Path $dist $f) -Algorithm SHA256).Hash.ToLower()
    "$h  $f"
}
Set-Content -Path (Join-Path $dist "SHA256SUMS.txt") -Value $lines -Encoding ascii

Write-Host ""
Write-Host "Bundle ready in $((Resolve-Path $dist).Path):"
Get-ChildItem $dist | Format-Table Name, Length -AutoSize | Out-String | Write-Host
& $picotool info -a (Join-Path $dist "cube-rs.uf2")
