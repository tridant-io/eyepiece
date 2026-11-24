# what stepping back costs on a long-group clip of your own, in the real program.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\backstep.ps1 -Clip a.mp4 -Frames 600 -KeyEvery 60
# goes to the middle of the last whole group, then presses F twenty times with no pause, timing each press until the
# title names the new frame without " . reading" (the picture is up). then twenty wheel notches back, sent at once.
# prints numbers; nothing is judged. a window opens.
param([Parameter(Mandatory = $true)][string]$Clip, [Parameter(Mandatory = $true)][int]$Frames, [Parameter(Mandatory = $true)][int]$KeyEvery,
    [string]$Exe)
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\eyepiece.exe' }
# players of this script's own: never handed to one the owner has open, and with settings of their own
$env:EYEPIECE_INSTANCE = "backstep-$PID"
$env:APPDATA = Join-Path $env:TEMP "eyepiece-backstep-$PID"
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class EyepieceBack {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
}
'@
function Get-Title { $s = New-Object Text.StringBuilder 512; [void][EyepieceBack]::GetWindowText($script:h, $s, 512); $s.ToString() }
# milliseconds until the title names this frame with its picture up, or -1 after 30 s
function Wait-Frame([int]$frame) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 30000) {
        $t = Get-Title
        if ($t -match " $frame / $Frames " -and $t -notmatch 'reading') { return [math]::Round($sw.Elapsed.TotalMilliseconds) }
        [Threading.Thread]::Sleep(1)
    }
    return -1
}
function Send-Wheel([int]$notches) { [void][EyepieceBack]::PostMessage($script:h, 0x20A, [IntPtr][int64]($notches * 120 * 65536), [IntPtr]0) }

$p = Start-Process $Exe -ArgumentList "`"$Clip`"" -PassThru
$script:h = [IntPtr]::Zero
foreach ($i in 1..100) { Start-Sleep -Milliseconds 100; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $script:h = $p.MainWindowHandle; break } }
Start-Sleep -Milliseconds 1500
$last = $Frames - 1
[int]$key = [math]::Floor(($last - 1) / $KeyEvery) * $KeyEvery
[int]$start = [math]::Min($last, $key + [math]::Floor($KeyEvery / 2))
[void][EyepieceBack]::PostMessage($script:h, 0x100, [IntPtr]0x23, [IntPtr]0); [void](Wait-Frame $last)
Send-Wheel ($last - $start)
"to frame $start, the middle of the group that starts at $key (the decoder walks $($start - $key) frames to get there): $(Wait-Frame $start) ms"
Start-Sleep -Milliseconds 1500

$times = foreach ($i in 1..20) { [void][EyepieceBack]::PostMessage($script:h, 0x100, [IntPtr]0x46, [IntPtr]0); Wait-Frame ($start - $i) }
"twenty presses of F, no pause, ms each: $($times -join ' ')"
"  slowest $(($times | Measure-Object -Maximum).Maximum) ms, total $(($times | Measure-Object -Sum).Sum) ms"
Start-Sleep -Milliseconds 1500

$from = $start - 20
$sw = [Diagnostics.Stopwatch]::StartNew()
foreach ($i in 1..20) { Send-Wheel 1 }
$settled = Wait-Frame ($from - 20)
"twenty wheel notches back, sent at once: picture up on frame $($from - 20) after $settled ms"
"title: $(Get-Title)"
[void]$p.CloseMainWindow(); if (-not $p.WaitForExit(3000)) { $p.Kill() }
# the script's own settings: the file, then the two folders made for it, each named before it goes. nothing recursive
foreach ($made in (Join-Path $env:APPDATA 'eyepiece\settings.txt'), (Join-Path $env:APPDATA 'eyepiece'), $env:APPDATA) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}
