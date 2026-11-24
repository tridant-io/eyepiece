# end to end: starts the real program on a tiny clip, drives it with real window messages and checks what it says.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\e2e.ps1 [-Exe path\to\eyepiece.exe]
# a window opens for a few seconds. exits 0 when every check holds. needs a desktop session.
# not covered here: dropping a file on the window.
param([string]$Exe = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\eyepiece.exe'))
$ErrorActionPreference = 'Stop'
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class EyepieceE2e {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int how);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
}
'@
$script:failed = 0
function Get-Title { $s = New-Object Text.StringBuilder 512; [void][EyepieceE2e]::GetWindowText($script:h, $s, 512); $s.ToString() }
function Send-Key([int]$vk) { [void][EyepieceE2e]::PostMessage($script:h, 0x100, [IntPtr]$vk, [IntPtr]0); Start-Sleep -Milliseconds 250 }
function Confirm-That([string]$what, [bool]$holds, [string]$detail = '') {
    if ($holds) { Write-Host "ok    $what" } else { Write-Host "FAIL  ${what}: $detail"; $script:failed++ }
}
# the title must come to match within two seconds; it is set by another thread
function Assert-Title([string]$what, [string]$pattern) {
    foreach ($i in 1..20) { if ((Get-Title) -match $pattern) { Write-Host "ok    $what"; return }; Start-Sleep -Milliseconds 100 }
    Confirm-That $what $false "the title is '$(Get-Title)', wanted /$pattern/"
}
function Start-Player([string]$clip) {
    $script:p = Start-Process $Exe -ArgumentList "`"$clip`"" -PassThru
    $script:h = [IntPtr]::Zero
    foreach ($i in 1..100) { Start-Sleep -Milliseconds 100; $script:p.Refresh(); if ($script:p.MainWindowHandle -ne [IntPtr]::Zero) { $script:h = $script:p.MainWindowHandle; return } }
    throw 'the player opened no window'
}
function Stop-Player {
    [void]$script:p.CloseMainWindow()
    if ($script:p.WaitForExit(3000)) { Confirm-That 'closes cleanly' ($script:p.ExitCode -eq 0) "exit code $($script:p.ExitCode)" }
    else { Confirm-That 'closes cleanly' $false 'still running after 3 s'; $script:p.Kill() }
}
function Get-Cpu { $script:p.Refresh(); $script:p.TotalProcessorTime.TotalSeconds }

$media = Join-Path $PSScriptRoot 'media'
Start-Player (Join-Path $media 'hapq_64.mov')
Assert-Title 'opens on frame 0 and reads the file exactly' 'hapq_64\.mov . 0 / 3 . 64 . 64 . hap q . 60000/1001 fps$'
Send-Key 0x47; Assert-Title 'G steps forward' ' 1 / 3 '
Send-Key 0x27; Assert-Title 'right arrow steps forward' ' 2 / 3 '
Send-Key 0x47; Assert-Title 'a step past the end stays on the last frame' ' 2 / 3 '
Send-Key 0x46; Assert-Title 'F steps back' ' 1 / 3 '
Send-Key 0x24; Assert-Title 'home goes to the first frame' ' 0 / 3 '
Send-Key 0x23; Assert-Title 'end goes to the last frame' ' 2 / 3 '

# the wheel: one notch toward the user is one frame forward, away is one back
Send-Key 0x24
[void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](-120 * 65536), [IntPtr]0); Assert-Title 'a wheel notch toward the user steps forward' ' 1 / 3 '
[void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](120 * 65536), [IntPtr]0); Assert-Title 'a wheel notch away steps back' ' 0 / 3 '

# the window changes size: the renderer has to follow and keep answering
[void][EyepieceE2e]::SetWindowPos($script:h, [IntPtr]::Zero, 0, 0, 900, 700, 0x0006); Start-Sleep -Milliseconds 300
Send-Key 0x47; Assert-Title 'still steps after the window is resized' ' 1 / 3 '
[void][EyepieceE2e]::ShowWindow($script:h, 3); Start-Sleep -Milliseconds 400
Send-Key 0x47; Assert-Title 'still steps when maximized' ' 2 / 3 '
[void][EyepieceE2e]::ShowWindow($script:h, 9); Start-Sleep -Milliseconds 400
Send-Key 0x46; Assert-Title 'still steps when restored' ' 1 / 3 '

# playing while minimized: no display to wait for, so the loop must pace itself and not spin a core
Send-Key 0x20
[void][EyepieceE2e]::ShowWindow($script:h, 6); Start-Sleep -Milliseconds 500
$before = Get-Cpu; Start-Sleep -Seconds 2; $used = (Get-Cpu) - $before
Confirm-That 'playing while minimized does not spin a core' ($used -lt 0.5) "used $([math]::Round($used, 2)) s of cpu in 2 s"
[void][EyepieceE2e]::ShowWindow($script:h, 9); Start-Sleep -Milliseconds 400
Send-Key 0x20
Assert-Title 'plays and pauses on a real frame, none unshowable' ' [0-2] / 3 . 64 . 64 . hap q . 60000/1001 fps( . \d+ late)?$'

# standing still costs nothing
$before = Get-Cpu; Start-Sleep -Seconds 2; $used = (Get-Cpu) - $before
Confirm-That 'standing still uses no cpu' ($used -lt 0.05) "used $([math]::Round($used, 2)) s of cpu in 2 s"
Stop-Player

# a clip ffmpeg decodes, with key frames four apart and frames stored out of order: steps go back across key frames
Start-Player (Join-Path $media 'h264_64.mp4')
Assert-Title 'h.264 opens on frame 0 and says how it is decoded' 'h264_64\.mp4 . 0 / 12 . 64 . 64 . h\.264, (hardware|software) . 15360/512 fps$'
Send-Key 0x23; Assert-Title 'h.264: end goes to the last frame' ' 11 / 12 '
Send-Key 0x46; Assert-Title 'h.264: a step back' ' 10 / 12 '
Send-Key 0x46; Send-Key 0x46; Send-Key 0x46; Assert-Title 'h.264: steps back across a key frame' ' 7 / 12 '
Send-Key 0x47; Assert-Title 'h.264: and forward again' ' 8 / 12 '
Send-Key 0x24; Assert-Title 'h.264: home goes to the first frame' ' 0 / 12 '
Send-Key 0x20; Start-Sleep -Milliseconds 1500; Send-Key 0x20
Assert-Title 'h.264 plays and pauses on a real frame, none unshowable' ' \d+ / 12 . 64 . 64 . h\.264, (hardware|software) . 15360/512 fps( . \d+ late)?$'
Stop-Player

Start-Player (Join-Path $media 'prores_64.mov')
Assert-Title 'prores opens, decoded in software' 'prores_64\.mov . 0 / 4 . 64 . 64 . prores, software . '
Send-Key 0x23; Assert-Title 'prores: end goes to the last frame' ' 3 / 4 '
Send-Key 0x46; Assert-Title 'prores: a step back' ' 2 / 4 '
Stop-Player

Start-Player (Join-Path $media 'hevc10_64.mp4')
Assert-Title 'hevc opens' 'hevc10_64\.mp4 . 0 / 12 . 64 . 64 . hevc, (hardware|software) . '
Send-Key 0x23; Assert-Title 'hevc: end goes to the last frame' ' 11 / 12 '
Send-Key 0x46; Send-Key 0x46; Assert-Title 'hevc: steps back' ' 9 / 12 '
Stop-Player

Start-Player (Join-Path $media 'no such clip.mov')
Assert-Title 'a missing file says so and the window stays' '^eyepiece . .*no such clip\.mov'
Stop-Player

if ($script:failed) { Write-Host "$($script:failed) check(s) failed"; exit 1 }
Write-Host 'all checks hold'
