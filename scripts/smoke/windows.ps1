# Smoke test on a Windows runner: runs the debug build and checks the CLI,
# single instance, windows, overlay and pasting into another app. Screenshots
# and logs land in smoke/ (uploaded as a CI artifact).
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path target/debug/bars.exe).Path
New-Item -ItemType Directory -Force smoke | Out-Null
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Fg {
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    public static string Title() { var s = new StringBuilder(256); GetWindowText(GetForegroundWindow(), s, 256); return s.ToString(); }
}
'@

function Shot($name) {
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
    $bmp.Save("smoke/$name.png")
    $g.Dispose(); $bmp.Dispose()
}

# Runs a `bars --flag` command and waits for it (GUI apps don't block).
function Bars([string]$flag) {
    $p = if ($flag) {
        Start-Process $exe -ArgumentList $flag -PassThru -Wait -NoNewWindow
    } else {
        Start-Process $exe -PassThru -Wait -NoNewWindow
    }
    if ($p.ExitCode -ne 0) { throw "bars $flag exited with $($p.ExitCode)" }
}

Write-Host '== doctor'
$doctor = & $exe --doctor | Out-String
$doctor | Set-Content smoke/doctor.txt
Write-Host $doctor
if ($doctor -notmatch 'Bars .* diagnostics') { throw 'doctor printed nothing' }

# One history entry for the paste test.
$data = Join-Path $env:APPDATA 'bars'
New-Item -ItemType Directory -Force $data | Out-Null
$entry = [ordered]@{ id = 1; at = (Get-Date).ToString('o'); text = 'Hello from the Bars smoke test'
    model = 'test'; audio_secs = 1.0; infer_ms = 1; words = 6 } | ConvertTo-Json -Compress
[IO.File]::WriteAllText((Join-Path $data 'history.jsonl'), "$entry`n")

Write-Host '== launch'
$env:RUST_LOG = 'info'
$app = Start-Process $exe -PassThru -RedirectStandardError smoke/bars.log -RedirectStandardOutput smoke/bars.out
Start-Sleep 12
if ($app.HasExited) { throw "bars exited early ($($app.ExitCode))" }
Shot '1-setup-assistant'

Write-Host '== second launch forwards to the running instance'
Bars ''
Start-Sleep 1
$running = @(Get-Process bars -ErrorAction SilentlyContinue).Count
if ($running -ne 1) { throw "expected 1 bars process, found $running" }

Write-Host '== settings'
Bars '--settings'
Start-Sleep 3
Shot '2-settings'

Write-Host '== overlay'
Bars '--toggle'
Start-Sleep 2
Shot '3-overlay'
Bars '--cancel'
Start-Sleep 1

Write-Host '== paste into another app'
$target = Start-Process pwsh -ArgumentList '-NoProfile', '-File', 'scripts/smoke/paste-target.ps1', 'smoke/pasted.txt' -PassThru
Start-Sleep 5
$fg = [Fg]::Title()
Write-Host "foreground window: '$fg'"
Bars '--paste-last'
Start-Sleep 2
Shot '4-pasted'
$null = $target.WaitForExit(20000)
$pasted = if (Test-Path smoke/pasted.txt) { Get-Content smoke/pasted.txt -Raw } else { '' }
Write-Host "pasted: '$pasted'"
$pasteOk = $pasted -match 'Hello from the Bars smoke test'

Write-Host '== quit'
Bars '--quit'
if (-not $app.WaitForExit(15000)) { throw 'bars did not quit' }

Write-Host '== log'
Get-Content smoke/bars.log

if (-not $pasteOk) {
    # Runners have no user input, so Windows may refuse to focus the target.
    if ($fg -eq 'Paste target') { throw 'paste-last did not reach the focused app' }
    Write-Warning "paste not verified: the target window never got focus (foreground was '$fg')"
}
Write-Host 'smoke test passed'
