# pacing check on real clips: plays each for some seconds in the real program and reads how many refreshes went by
# without the frame that was due (the title says " . n late" once the clip is paused).
#   powershell -NoProfile -ExecutionPolicy Bypass -Command "& tests\play.ps1 -Clips a.mov,b.mov [-Seconds 10] [-MaxLate 5]"
# big clips are not in the repo: pass your own. a window opens for each. exits 1 if any clip is later than -MaxLate.
param([Parameter(Mandatory = $true)][string[]]$Clips, [int]$Seconds = 10, [int]$MaxLate = 5,
    [string]$Exe = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\eyepiece.exe'))
$ErrorActionPreference = 'Stop'
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class EyepiecePlay {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
}
'@
function Get-Title($h) { $s = New-Object Text.StringBuilder 512; [void][EyepiecePlay]::GetWindowText($h, $s, 512); $s.ToString() }
$failed = 0
foreach ($clip in $Clips) {
    $p = Start-Process $Exe -ArgumentList "`"$clip`"" -PassThru
    $h = [IntPtr]::Zero
    foreach ($i in 1..100) { Start-Sleep -Milliseconds 100; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break } }
    if ($h -eq [IntPtr]::Zero) { Write-Host "FAIL  ${clip}: no window"; $failed++; continue }
    Start-Sleep -Milliseconds 1000
    [void][EyepiecePlay]::PostMessage($h, 0x100, [IntPtr]0x20, [IntPtr]0)
    Start-Sleep -Seconds $Seconds
    [void][EyepiecePlay]::PostMessage($h, 0x100, [IntPtr]0x20, [IntPtr]0)
    Start-Sleep -Milliseconds 500
    $title = Get-Title $h
    $late = if ($title -match ' (\d+) late$') { [int]$Matches[1] } else { 0 }
    $p.Refresh()
    $verdict = if ($late -le $MaxLate) { 'ok  ' } else { $failed++; 'FAIL' }
    Write-Host "$verdict  $late late in $Seconds s, $([math]::Round($p.WorkingSet64 / 1MB)) MB   $title"
    [void]$p.CloseMainWindow(); if (-not $p.WaitForExit(3000)) { $p.Kill() }
}
if ($failed) { exit 1 }
