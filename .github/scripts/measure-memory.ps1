<#
  Records the memory use of a built Snapline exe at fixed points and writes
  the raw numbers to a JSON file. It never fails on the numbers themselves.
  Run it the same way for every release so the figures are comparable.

  Stages: idle (tray only), main_open, main_closed, overlay (region capture
  overlay showing), overlay_closed. Numbers are bytes of the Snapline process:
  private_ws = private working set, private_bytes = committed private memory,
  working_set = full working set.
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out,
    [string]$Version = "dev",
    [string]$Commit = "",
    [string]$Profile = "release"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms

$stage = Join-Path ([IO.Path]::GetTempPath()) "snapline-memory"
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item $stage -ItemType Directory | Out-Null
Copy-Item $Exe "$stage\snapline.exe"
New-Item "$stage\portable" -ItemType File | Out-Null
$app = "$stage\snapline.exe"

function Log($m) { Write-Host ("[{0}] {1}" -f (Get-Date -Format HH:mm:ss), $m) }

function Read-Once($procId) {
    $row = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$procId"
    if (-not $row) { return $null }
    [pscustomobject]@{
        private_ws    = [int64]$row.WorkingSetPrivate
        private_bytes = [int64]$row.PrivateBytes
        working_set   = [int64]$row.WorkingSet
    }
}

# Median of five readings one second apart, per field.
function Sample($procId) {
    $rows = @()
    for ($i = 0; $i -lt 5; $i++) {
        $r = Read-Once $procId
        if ($r) { $rows += $r }
        Start-Sleep -Seconds 1
    }
    if ($rows.Count -eq 0) { return $null }
    $median = {
        param($field)
        $v = @($rows | ForEach-Object { $_.$field } | Sort-Object)
        $v[[int][Math]::Floor($v.Count / 2)]
    }
    [ordered]@{
        private_ws    = & $median "private_ws"
        private_bytes = & $median "private_bytes"
        working_set   = & $median "working_set"
    }
}

function Visible-Windows($procId) {
    @(Get-Process -Id $procId -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 })
}

function Wait-Windows($procId, [bool]$want, $timeoutSec) {
    $end = (Get-Date).AddSeconds($timeoutSec)
    while ((Get-Date) -lt $end) {
        if ((Visible-Windows $procId).Count -gt 0 -eq $want) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

$stages = [ordered]@{}
$notes = @()
$proc = Start-Process $app -ArgumentList "--budget-run" -PassThru
Log "started pid $($proc.Id)"
try {
    Start-Sleep -Seconds 20
    if ($proc.HasExited) { throw "app exited during startup (code $($proc.ExitCode))" }
    $stages["idle"] = Sample $proc.Id
    Log "idle: $($stages['idle'].private_ws)"

    Start-Process $app -ArgumentList "--budget-open", "main" | Out-Null
    if (-not (Wait-Windows $proc.Id $true 30)) { $notes += "main window did not appear" }
    Start-Sleep -Seconds 3
    $stages["main_open"] = Sample $proc.Id
    Log "main_open: $($stages['main_open'].private_ws)"

    $null = (Get-Process -Id $proc.Id).CloseMainWindow()
    if (-not (Wait-Windows $proc.Id $false 15)) { $notes += "main window did not close" }
    Start-Sleep -Seconds 5
    $stages["main_closed"] = Sample $proc.Id
    Log "main_closed: $($stages['main_closed'].private_ws)"

    Start-Process $app -ArgumentList "--action", "capture_region" | Out-Null
    if (Wait-Windows $proc.Id $true 15) {
        Start-Sleep -Seconds 2
        $stages["overlay"] = Sample $proc.Id
        Log "overlay: $($stages['overlay'].private_ws)"
        [Windows.Forms.SendKeys]::SendWait("{ESC}")
        if (-not (Wait-Windows $proc.Id $false 15)) { $notes += "overlay did not close on Esc" }
        Start-Sleep -Seconds 5
        $stages["overlay_closed"] = Sample $proc.Id
        Log "overlay_closed: $($stages['overlay_closed'].private_ws)"
    } else {
        $notes += "capture overlay did not appear"
    }
} finally {
    Start-Process $app -ArgumentList "--action", "app_exit" | Out-Null
    if (-not $proc.WaitForExit(10000)) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
}

$os = Get-CimInstance Win32_OperatingSystem
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$screen = [Windows.Forms.SystemInformation]::PrimaryMonitorSize
$run = [ordered]@{
    version = $Version
    commit  = $Commit
    date    = (Get-Date).ToUniversalTime().ToString("yyyy-MM-dd")
    profile = $Profile
    runner  = [ordered]@{
        os           = $os.Caption + " " + $os.Version
        cpu          = $cpu.Name.Trim()
        logical_cpus = [int]$env:NUMBER_OF_PROCESSORS
        ram_gb       = [Math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
        screen       = "$($screen.Width)x$($screen.Height)"
    }
    method  = "median of 5 readings 1 s apart per stage; Win32_PerfFormattedData_PerfProc_Process of the snapline.exe process"
    stages  = $stages
    notes   = $notes
}
$run | ConvertTo-Json -Depth 6 | Set-Content $Out -Encoding utf8
Log "wrote $Out"
if ($notes.Count -gt 0) { $notes | ForEach-Object { Write-Host "::warning::$_" } }
