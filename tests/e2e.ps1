# end to end: starts the real program on a tiny clip, drives it with real key messages and checks what it says.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\e2e.ps1 [-Exe path\to\eyepiece.exe]
# a window opens for a few seconds. exits 0 when every check holds. needs a desktop session.
param([string]$Exe = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\eyepiece.exe'))
$ErrorActionPreference = 'Stop'
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class EyepieceE2e {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
}
'@
$script:failed = 0
function Get-Title { $s = New-Object Text.StringBuilder 512; [void][EyepieceE2e]::GetWindowText($script:h, $s, 512); $s.ToString() }
function Send-Key([int]$vk) { [void][EyepieceE2e]::PostMessage($script:h, 0x100, [IntPtr]$vk, [IntPtr]0); Start-Sleep -Milliseconds 250 }
# the title must come to match within two seconds; it is set by another thread
function Assert-Title([string]$what, [string]$pattern) {
    foreach ($i in 1..20) { if ((Get-Title) -match $pattern) { Write-Host "ok    $what"; return }; Start-Sleep -Milliseconds 100 }
    Write-Host "FAIL  ${what}: the title is '$(Get-Title)', wanted /$pattern/"; $script:failed++
}
function Start-Player([string]$clip) {
    $script:p = Start-Process $Exe -ArgumentList "`"$clip`"" -PassThru
    $script:h = [IntPtr]::Zero
    foreach ($i in 1..100) { Start-Sleep -Milliseconds 100; $script:p.Refresh(); if ($script:p.MainWindowHandle -ne [IntPtr]::Zero) { $script:h = $script:p.MainWindowHandle; return } }
    throw 'the player opened no window'
}
function Stop-Player {
    [void]$script:p.CloseMainWindow()
    if ($script:p.WaitForExit(3000)) { if ($script:p.ExitCode -ne 0) { Write-Host "FAIL  exit code $($script:p.ExitCode)"; $script:failed++ } else { Write-Host 'ok    closes cleanly' } }
    else { Write-Host 'FAIL  did not close in 3 s'; $script:failed++; $script:p.Kill() }
}

$media = Join-Path $PSScriptRoot 'media'
Start-Player (Join-Path $media 'hapq_64.mov')
Assert-Title 'opens on frame 0 and reads the file exactly' 'hapq_64\.mov . 0 / 3 . 64 . 64 . hap q . 60000/1001 fps$'
Send-Key 0x47; Assert-Title 'G steps forward' ' 1 / 3 '
Send-Key 0x27; Assert-Title 'right arrow steps forward' ' 2 / 3 '
Send-Key 0x47; Assert-Title 'a step past the end stays on the last frame' ' 2 / 3 '
Send-Key 0x46; Assert-Title 'F steps back' ' 1 / 3 '
Send-Key 0x24; Assert-Title 'home goes to the first frame' ' 0 / 3 '
Send-Key 0x23; Assert-Title 'end goes to the last frame' ' 2 / 3 '
Send-Key 0x20; Start-Sleep -Milliseconds 1500; Send-Key 0x20
Assert-Title 'plays and pauses on a real frame, none unshowable' ' [0-2] / 3 . 64 . 64 . hap q . 60000/1001 fps( . \d+ late)?$'
Stop-Player

Start-Player (Join-Path $media 'no such clip.mov')
Assert-Title 'a missing file says so and the window stays' '^eyepiece . .*no such clip\.mov'
Stop-Player

if ($script:failed) { Write-Host "$($script:failed) check(s) failed"; exit 1 }
Write-Host 'all checks hold'
