# Make the cube terminal open by itself whenever the cube is plugged in.
#   powershell -ExecutionPolicy Bypass -File tools\install_autostart.ps1           install (and start now)
#   powershell -ExecutionPolicy Bypass -File tools\install_autostart.ps1 -Remove   uninstall (and stop it)
# It adds a shortcut to cube_autostart.pyw in your Windows Startup folder; nothing else on the system changes.
param([switch]$Remove)
$ErrorActionPreference = 'Stop'

$startup = [Environment]::GetFolderPath('Startup')
$lnk = Join-Path $startup 'TTED Cube autostart.lnk'
$script = Join-Path $PSScriptRoot 'cube_autostart.pyw'

# Stop a running watcher (pythonw running cube_autostart.pyw).
Get-CimInstance Win32_Process -Filter "Name='pythonw.exe'" |
    Where-Object { $_.CommandLine -like '*cube_autostart.pyw*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }

if ($Remove) {
    if (Test-Path $lnk) { Remove-Item $lnk }
    Write-Host "Removed: the cube terminal no longer opens on plug-in."
    return
}

# pythonw.exe next to the real python.exe (not the Microsoft Store stub).
$py = & python -c "import sys; print(sys.executable)"
$pyw = Join-Path (Split-Path $py) 'pythonw.exe'
if (-not (Test-Path $pyw)) { throw "pythonw.exe not found next to $py" }
& $py -c "import serial" 2>$null
if ($LASTEXITCODE -ne 0) { throw "pyserial is missing: pip install pyserial" }

$ws = New-Object -ComObject WScript.Shell
$s = $ws.CreateShortcut($lnk)
$s.TargetPath = $pyw
$s.Arguments = "`"$script`""
$s.WorkingDirectory = $PSScriptRoot
$s.Description = 'Opens the TTED cube terminal when the cube is plugged in'
$s.Save()

Start-Process -FilePath $pyw -ArgumentList "`"$script`"" -WorkingDirectory $PSScriptRoot
Write-Host "Installed: $lnk"
Write-Host "The watcher is running now and starts at every login. Plug in the cube and the terminal opens."
