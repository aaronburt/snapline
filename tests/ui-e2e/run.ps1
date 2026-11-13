<#
  Scripted end-to-end UI run for SnapLine on Windows.
  Launches the real exe in portable mode (throwaway data), drives it through
  Windows UI Automation, screenshots every stage, and fails on any crash or
  missing control.  Output: $OutDir\*.png, *.tree.txt, run.log
#>
param(
    [string]$Exe = "target\debug\snapline.exe",
    [string]$OutDir = "target\ui-e2e"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Win { [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }
"@
[Win]::SetProcessDPIAware() | Out-Null

$OutDir = [IO.Path]::GetFullPath($OutDir)
Remove-Item $OutDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item $OutDir -ItemType Directory | Out-Null
$script:step = 0
$script:failures = @()
function Log($m) { $l = "[{0}] {1}" -f (Get-Date -Format HH:mm:ss), $m; Write-Host $l; Add-Content "$OutDir\run.log" $l }

function Shot($name) {
    $script:step++
    $file = "{0:D2}-{1}" -f $script:step, ($name -replace '[^\w\-]+', '-')
    try {
        $r = $script:win.Current.BoundingRectangle
        if ($r.Width -lt 10) { throw "window has no size" }
        $bmp = New-Object Drawing.Bitmap ([int]$r.Width), ([int]$r.Height)
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen([int]$r.X, [int]$r.Y, 0, 0, $bmp.Size)
        $bmp.Save("$OutDir\$file.png", [Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    } catch {
        # fall back to full screen so a failure still leaves evidence
        $b = [Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size)
        $bmp.Save("$OutDir\$file.png", [Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    }
    # element tree beside the picture: names, roles, positions (useful for AI review)
    try {
        $all = $script:win.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
        $lines = foreach ($e in $all) {
            $c = $e.Current; $b = $c.BoundingRectangle
            "{0}`t{1}`t{2}`t{3},{4} {5}x{6}`tenabled={7}" -f $c.ControlType.ProgrammaticName, $c.Name, $c.AutomationId, [int]$b.X, [int]$b.Y, [int]$b.Width, [int]$b.Height, $c.IsEnabled
        }
        $lines | Set-Content "$OutDir\$file.tree.txt"
    } catch { Log "tree dump failed: $_" }
    Log "screenshot $file"
}

function Fail($m) { Log "FAIL: $m"; $script:failures += $m }

function Alive { if ($script:proc.HasExited) { Fail "app exited (code $($script:proc.ExitCode))"; return $false }; $true }

function Find($name, $timeoutSec = 8) {
    $cond = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::NameProperty), $name
    $end = (Get-Date).AddSeconds($timeoutSec)
    do {
        $e = $script:win.FindFirst([Windows.Automation.TreeScope]::Descendants, $cond)
        if ($e) { return $e }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $end -and -not $script:proc.HasExited)
    return $null
}

function Press($name, $shotName) {
    if (-not (Alive)) { return $false }
    $e = Find $name
    if (-not $e) { Fail "control not found: '$name'"; Shot "missing-$name"; return $false }
    try {
        $invoke = $e.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
        $invoke.Invoke()
    } catch {
        # no Invoke pattern: click the centre of its box
        $r = $e.Current.BoundingRectangle
        [Windows.Forms.Cursor]::Position = New-Object Drawing.Point ([int]($r.X + $r.Width / 2)), ([int]($r.Y + $r.Height / 2))
        Add-Type -Name Mouse -Namespace Native -MemberDefinition '[DllImport("user32.dll")] public static extern void mouse_event(int f,int x,int y,int d,int e);' -ErrorAction SilentlyContinue
        [Native.Mouse]::mouse_event(2,0,0,0,0); [Native.Mouse]::mouse_event(4,0,0,0,0)
    }
    Start-Sleep -Milliseconds 700
    Shot $(if ($shotName) { $shotName } else { $name })
    return (Alive)
}

# --- launch in portable mode so nothing touches a real profile ---
$stage = Join-Path $env:RUNNER_TEMP "snapline-e2e"
if (-not $env:RUNNER_TEMP) { $stage = Join-Path ([IO.Path]::GetTempPath()) "snapline-e2e" }
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item $stage -ItemType Directory | Out-Null
Copy-Item $Exe "$stage\snapline.exe"
if (Test-Path "third_party\ffmpeg\ffmpeg.exe") { Copy-Item "third_party\ffmpeg\ffmpeg.exe" $stage }
New-Item "$stage\portable" -ItemType File | Out-Null

Log "launching $stage\snapline.exe"
$script:proc = Start-Process "$stage\snapline.exe" -ArgumentList "--budget-open", "main" -PassThru

# --- find the main window ---
$script:win = $null
$cond = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ProcessIdProperty), $script:proc.Id
$end = (Get-Date).AddSeconds(45)
while ((Get-Date) -lt $end -and -not $script:win) {
    if ($script:proc.HasExited) { break }
    foreach ($w in [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $cond)) {
        if ($w.Current.Name -eq "Snapline" -and $w.Current.BoundingRectangle.Width -gt 100) { $script:win = $w; break }
    }
    Start-Sleep -Milliseconds 500
}
if (-not $script:win) {
    Log "main window never appeared (exited=$($script:proc.HasExited))"
    $script:win = [Windows.Automation.AutomationElement]::RootElement
    Shot "no-window"
    exit 1
}
try { [Win]::SetForegroundWindow([IntPtr]$script:win.Current.NativeWindowHandle) | Out-Null } catch {}
Start-Sleep -Seconds 1
Shot "launch"

# --- pages: every sidebar entry, screenshot after each ---
foreach ($page in "General", "Capture", "Hotkeys", "Upload destinations", "History", "Workflows", "Tools") {
    Press $page "page-$page" | Out-Null
}

# --- Upload destinations: open the add menu and every form, then cancel out ---
Press "Upload destinations" "uploads-again" | Out-Null
if (Press "Add destination" "add-menu-open") {
    foreach ($kind in "Amazon S3", "GitHub Gist", "SFTP", "Custom uploader") {
        if (-not (Find $kind 2)) { if (-not (Press "Add destination" "add-menu-$kind")) { break } }
        if (Press $kind "form-$kind") {
            if (Find "Cancel" 2) { Press "Cancel" "form-$kind-cancelled" | Out-Null }
            if (-not (Find "Add destination" 2)) { Press "Upload destinations" "uploads-back" | Out-Null }
            Press "Add destination" "add-menu-reopen" | Out-Null
        }
    }
}

# --- Workflows: open the new-workflow editor and cancel ---
Press "Workflows" "workflows-again" | Out-Null
if (Press "New workflow") { if (Find "Cancel" 3) { Press "Cancel" "workflow-editor-cancelled" | Out-Null } }

# --- Tools page ---
Press "Tools" "tools-again" | Out-Null
Shot "tools-final"

# --- end: app must still be running ---
if (Alive) {
    Log "app still running after the full run"
    Shot "final"
    try { $script:proc.CloseMainWindow() | Out-Null; Start-Sleep 2 } catch {}
    if (-not $script:proc.HasExited) { Stop-Process $script:proc -Force }
}

if ($script:failures.Count -gt 0) {
    Log ("{0} failure(s):" -f $script:failures.Count)
    $script:failures | ForEach-Object { Log " - $_" }
    exit 1
}
Log "all checks passed"
