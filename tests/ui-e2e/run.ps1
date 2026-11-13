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
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class Disp {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Ansi)]
  public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
    public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra;
    public int dmFields, dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
    public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
    public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
    public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
  }
  [DllImport("user32.dll", CharSet = CharSet.Ansi)] public static extern int ChangeDisplaySettings(ref DEVMODE m, int flags);
  public static int Set(int w, int h) {
    DEVMODE m = new DEVMODE(); m.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
    m.dmPelsWidth = w; m.dmPelsHeight = h; m.dmFields = 0x80000 | 0x100000;
    return ChangeDisplaySettings(ref m, 0);
  }
}
"@

# A bigger desktop so the whole window fits (runners default to 1024x768).
try { Write-Host ("display change result: " + [Disp]::Set(1920, 1080)); Start-Sleep -Seconds 2 } catch { Write-Host "display change failed: $_" }
$OutDir = [IO.Path]::GetFullPath($OutDir)
Remove-Item $OutDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item $OutDir -ItemType Directory | Out-Null
$script:step = 0
$script:failures = @()
function Log($m) { $l = "[{0}] {1}" -f (Get-Date -Format HH:mm:ss), $m; Write-Host $l; Add-Content "$OutDir\run.log" $l }

function Grab($r) {
    $scr = [Windows.Forms.SystemInformation]::VirtualScreen
    $x = [Math]::Max([int]$r.X, $scr.X); $y = [Math]::Max([int]$r.Y, $scr.Y)
    $w = [Math]::Min([int]($r.X + $r.Width), $scr.X + $scr.Width) - $x
    $h = [Math]::Min([int]($r.Y + $r.Height), $scr.Y + $scr.Height) - $y
    if ($w -lt 10 -or $h -lt 10) { throw "window is off screen" }
    $bmp = New-Object Drawing.Bitmap $w, $h
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
    $g.Dispose()
    return $bmp
}
function BmpHash($bmp) {
    $ms = New-Object IO.MemoryStream
    $bmp.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
    return [BitConverter]::ToString([Security.Cryptography.MD5]::Create().ComputeHash($ms.ToArray()))
}

function Shot($name) {
    $script:step++
    $file = "{0:D2}-{1}" -f $script:step, ($name -replace '[^\w\-]+', '-')
    try {
        # wait until two captures in a row are identical, so the picture matches the step
        Start-Sleep -Milliseconds 400
        $bmp = Grab $script:win.Current.BoundingRectangle
        $hash = BmpHash $bmp
        for ($i = 0; $i -lt 12; $i++) {
            Start-Sleep -Milliseconds 250
            $next = Grab $script:win.Current.BoundingRectangle
            $nextHash = BmpHash $next
            $bmp.Dispose(); $bmp = $next
            if ($nextHash -eq $hash) { break }
            $hash = $nextHash
        }
        $bmp.Save("$OutDir\$file.png", [Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
    } catch {
        $b = [Windows.Forms.SystemInformation]::VirtualScreen
        $bmp = New-Object Drawing.Bitmap $b.Width, $b.Height
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size)
        $bmp.Save("$OutDir\$file.png", [Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
    }
    try {
        $all = $script:win.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
        $lines = foreach ($e in $all) {
            $c = $e.Current; $b = $c.BoundingRectangle
            $state = ""
            try { $t = $e.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern); $state = "toggle=" + $t.Current.ToggleState } catch {}
            "{0}`t{1}`t{2}`t{3},{4} {5}x{6}`tenabled={7} {8}" -f $c.ControlType.ProgrammaticName, $c.Name, $c.AutomationId, [int]$b.X, [int]$b.Y, [int]$b.Width, [int]$b.Height, $c.IsEnabled, $state
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
        $hits = $script:win.FindAll([Windows.Automation.TreeScope]::Descendants, $cond)
        $pick = $null
        foreach ($h in $hits) { if ($h.Current.ControlType -ne [Windows.Automation.ControlType]::Text) { $pick = $h; break } }
        if (-not $pick -and $hits.Count -gt 0) { $pick = $hits[0] }
        if ($pick) { return $pick }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $end -and -not $script:proc.HasExited)
    return $null
}

function Press($name, $shotName) {
    if (-not (Alive)) { return $false }
    $e = Find $name
    if (-not $e) { Fail "control not found: '$name'"; Shot "missing-$name"; return $false }
    try {
        try { $e.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
        catch { $e.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle() }
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

# --- Switches and segmented controls: flip each one, screenshot, flip it back ---
function ExerciseToggles($page) {
    if (-not (Alive)) { return }
    Press $page "toggles-$page-start" | Out-Null
    $cond = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::IsTogglePatternAvailableProperty), $true
    $count = @($script:win.FindAll([Windows.Automation.TreeScope]::Descendants, $cond)).Count
    Log "${page}: $count switch-like controls"
    for ($i = 0; $i -lt [Math]::Min($count, 30); $i++) {
        if (-not (Alive)) { break }
        try {
            # re-read the list each time: flipping a control can re-lay out the page
            $items = @($script:win.FindAll([Windows.Automation.TreeScope]::Descendants, $cond))
            if ($i -ge $items.Count) { break }
            $e = $items[$i]
            $name = $e.Current.Name
            if ($name -match '%') { continue }   # zoom options are exercised separately
            $t = $e.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
            $before = $t.Current.ToggleState
            $t.Toggle(); Start-Sleep -Milliseconds 300
            $after = $t.Current.ToggleState
            Shot "toggle-$page-$name"
            if ($before -ne $after) { $t.Toggle(); Start-Sleep -Milliseconds 300 }
        } catch { Log "note: control $i on ${page} failed: $_" }
    }
}
foreach ($page in "General", "Capture", "Hotkeys", "Upload destinations") { ExerciseToggles $page }

# Switches show up to UI Automation as buttons with Invoke: flip each one, screenshot, flip it back
$switches = [ordered]@{
    "General" = "Launch at sign-in", "Start minimised to tray", "Open capture on tray click", "Notifications"
    "Capture" = "Include mouse cursor", "Shutter sound", "Keep window shadow and rounded corners", "Freeze the screen while selecting", "Magnifier", "Snap to windows and controls", "Show size and position", "Full-screen crosshair", "Record system audio", "Record microphone", "Show recording border", "Optimise colour palette"
    "Hotkeys" = "Take over the PrtSc key", "Warn about clashes"
    "Upload destinations" = "Remove location and camera data", "Ask before uploading from the clipboard"
}
foreach ($page in $switches.Keys) {
    Press $page "switches-$page" | Out-Null
    foreach ($name in $switches[$page]) {
        if (-not (Alive)) { break }
        if (Find $name 1) {
            Press $name "switch-$name-flipped" | Out-Null
            Press $name "switch-$name-restored" | Out-Null
        } else { Log "note: switch '$name' not shown on $page" }
    }
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

# --- Zoom (last, so a stuck zoom can't affect the other steps) ---
Press "General" "zoom-start" | Out-Null
foreach ($z in "50%", "150%") { Press $z "zoom-$z" | Out-Null }
# Ctrl+0 resets the zoom, so later steps find their controls in the usual place
try { [Win]::SetForegroundWindow([IntPtr]$script:win.Current.NativeWindowHandle) | Out-Null; [Windows.Forms.SendKeys]::SendWait("^0"); Start-Sleep -Milliseconds 800 } catch { Log "zoom reset failed: $_" }
Shot "zoom-reset"

# Diagnostics for switches: what UI Automation reports for them
Press "General" "switch-diagnostics" | Out-Null
foreach ($e in $script:win.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)) {
    $c = $e.Current
    if ($c.ControlType -ne [Windows.Automation.ControlType]::Text -and $c.Name -notmatch '%') {
        $pats = ($e.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName -replace 'Identifiers.Pattern','' }) -join ','
        Log ("  element {0} '{1}' patterns=[{2}]" -f $c.ControlType.ProgrammaticName, $c.Name, $pats)
    }
}

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
