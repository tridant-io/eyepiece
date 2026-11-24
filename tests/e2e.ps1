# end to end: starts the real program on tiny clips, drives it with real window messages and checks what it says.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\e2e.ps1 [-Exe path\to\eyepiece.exe]
# windows open for a few seconds each. exits 0 when every check holds. needs a desktop session and a gpu with a
# video decoder (the h.264 and hevc checks require hardware decoding).
# not covered here: dropping a file on the window, and shift with a key.
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
function Send-Key([int]$vk, [int]$times = 1) { foreach ($i in 1..$times) { [void][EyepieceE2e]::PostMessage($script:h, 0x100, [IntPtr]$vk, [IntPtr]0); Start-Sleep -Milliseconds 200 } }
function Confirm-That([string]$what, [bool]$holds, [string]$detail = '') {
    if ($holds) { Write-Host "ok    $what" } else { Write-Host "FAIL  ${what}: $detail"; $script:failed++ }
}
# the title is set by another thread: it must come to satisfy the test within two seconds
function Wait-Title([string]$what, [scriptblock]$test, [string]$wanted) {
    foreach ($i in 1..20) { if (& $test (Get-Title)) { Write-Host "ok    $what"; return }; Start-Sleep -Milliseconds 100 }
    Confirm-That $what $false "the title is '$(Get-Title)', wanted $wanted"
}
function Assert-Title([string]$what, [string]$pattern) { Wait-Title $what { param($t) $t -match $pattern }.GetNewClosure() "/$pattern/" }
# the clip is open on exactly this frame, nothing about it could not be shown, and (when asked) it is decoded this way
function Assert-Frame([string]$what, [int]$frame, [int]$count, [string]$decoded = '') {
    $start = [regex]::Escape($script:name) + " . $frame / $count . "
    Wait-Title $what { param($t) $t -match "^$start" -and $t -match 'fps( . \d+ late)?$' -and ($decoded -eq '' -or $t -match ", $decoded . ") }.GetNewClosure() "frame $frame of $count, no fault$(if ($decoded) { ", $decoded" })"
}
function Start-Player([string]$clip) {
    $script:name = Split-Path $clip -Leaf
    $script:p = Start-Process $Exe -ArgumentList "`"$clip`"" -PassThru
    $script:h = [IntPtr]::Zero
    foreach ($i in 1..100) { Start-Sleep -Milliseconds 100; $script:p.Refresh(); if ($script:p.MainWindowHandle -ne [IntPtr]::Zero) { $script:h = $script:p.MainWindowHandle; return } }
    throw 'the player opened no window'
}
function Stop-Player {
    [void]$script:p.CloseMainWindow()
    if ($script:p.WaitForExit(3000)) { Confirm-That "$($script:name) closes cleanly" ($script:p.ExitCode -eq 0) "exit code $($script:p.ExitCode)" }
    else { Confirm-That "$($script:name) closes cleanly" $false 'still running after 3 s'; $script:p.Kill() }
}
function Get-Cpu { $script:p.Refresh(); $script:p.TotalProcessorTime.TotalSeconds }

$media = Join-Path $PSScriptRoot 'media'

# ---- hap: our own lane, and everything about the window
Start-Player (Join-Path $media 'hapq_64.mov')
Assert-Title 'hap q opens on frame 0 and reads the file exactly' '^hapq_64\.mov . 0 / 3 . 64 . 64 . hap q . 60000/1001 fps$'
Send-Key 0x47; Assert-Frame 'G steps forward' 1 3
Send-Key 0x27; Assert-Frame 'right arrow steps forward' 2 3
Send-Key 0x47; Assert-Frame 'a step past the end stays on the last frame' 2 3
Send-Key 0x46; Assert-Frame 'F steps back' 1 3
Send-Key 0x24; Assert-Frame 'home goes to the first frame' 0 3
Send-Key 0x23; Assert-Frame 'end goes to the last frame' 2 3
# the wheel: one notch toward the user is one frame forward, away is one back
Send-Key 0x24
[void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](-120 * 65536), [IntPtr]0); Assert-Frame 'a wheel notch toward the user steps forward' 1 3
[void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](120 * 65536), [IntPtr]0); Assert-Frame 'a wheel notch away steps back' 0 3
# the window changes size: the renderer has to follow and keep answering
[void][EyepieceE2e]::SetWindowPos($script:h, [IntPtr]::Zero, 0, 0, 900, 700, 0x0006); Start-Sleep -Milliseconds 300
Send-Key 0x47; Assert-Frame 'still steps after the window is resized' 1 3
[void][EyepieceE2e]::ShowWindow($script:h, 3); Start-Sleep -Milliseconds 400
Send-Key 0x47; Assert-Frame 'still steps when maximized' 2 3
[void][EyepieceE2e]::ShowWindow($script:h, 9); Start-Sleep -Milliseconds 400
Send-Key 0x46; Assert-Frame 'still steps when restored' 1 3
# playing while minimized: no display to wait for, so the loop must pace itself and not spin a core
Send-Key 0x20
[void][EyepieceE2e]::ShowWindow($script:h, 6); Start-Sleep -Milliseconds 500
$before = Get-Cpu; Start-Sleep -Seconds 2; $used = (Get-Cpu) - $before
Confirm-That 'playing while minimized does not spin a core' ($used -lt 0.5) "used $([math]::Round($used, 2)) s of cpu in 2 s"
[void][EyepieceE2e]::ShowWindow($script:h, 9); Start-Sleep -Milliseconds 400
Send-Key 0x20
Assert-Title 'plays and pauses on a real frame, none unshowable' '^hapq_64\.mov . [0-2] / 3 . 64 . 64 . hap q . 60000/1001 fps( . \d+ late)?$'
$before = Get-Cpu; Start-Sleep -Seconds 2; $used = (Get-Cpu) - $before
Confirm-That 'standing still uses no cpu' ($used -lt 0.05) "used $([math]::Round($used, 2)) s of cpu in 2 s"
Stop-Player

# ---- h.264 in hardware: key frames eight apart, frames stored out of display order
Start-Player (Join-Path $media 'h264_320.mp4')
Assert-Frame 'h.264 opens on frame 0, decoded by the gpu' 0 24 'hardware'
Send-Key 0x23; Assert-Frame 'h.264: end goes to the last frame' 23 24 'hardware'
Send-Key 0x46 8; Assert-Frame 'h.264: eight steps back, across a key frame' 15 24 'hardware'
Send-Key 0x47 2; Assert-Frame 'h.264: and forward again, across it' 17 24 'hardware'
Send-Key 0x24; Assert-Frame 'h.264: home goes to the first frame' 0 24 'hardware'
Send-Key 0x20; Start-Sleep -Milliseconds 1500; Send-Key 0x20
Assert-Title 'h.264 plays and pauses on a real frame, none unshowable' '^h264_320\.mp4 . \d+ / 24 . .*, hardware . 15360/512 fps( . \d+ late)?$'
Stop-Player

# ---- 10-bit hevc in hardware: its key frames have pictures that are shown before them
Start-Player (Join-Path $media 'hevc10_320.mp4')
Assert-Frame 'hevc opens on frame 0, decoded by the gpu' 0 24 'hardware'
Send-Key 0x23; Assert-Frame 'hevc: end goes to the last frame' 23 24 'hardware'
Send-Key 0x46 9; Assert-Frame 'hevc: nine steps back, across a key frame' 14 24 'hardware'
Send-Key 0x24; Assert-Frame 'hevc: home goes to the first frame' 0 24 'hardware'
Stop-Player

# ---- a stream the gpu does not take (4:2:2, 10-bit): the same clip decodes in software
Start-Player (Join-Path $media 'h264_422_64.mp4')
Assert-Frame 'h.264 4:2:2 opens, decoded in software' 0 6 'software'
Send-Key 0x23; Assert-Frame 'h.264 4:2:2: end goes to the last frame' 5 6 'software'
Send-Key 0x46 3; Assert-Frame 'h.264 4:2:2: steps back across a key frame' 2 6 'software'
Stop-Player

# ---- prores: software, every frame stands alone
Start-Player (Join-Path $media 'prores_64.mov')
Assert-Frame 'prores opens, decoded in software' 0 4 'software'
Send-Key 0x23; Assert-Frame 'prores: end goes to the last frame' 3 4 'software'
Send-Key 0x46; Assert-Frame 'prores: a step back' 2 4 'software'
Stop-Player

# ---- what cannot be opened says why, and the window stays
Start-Player (Join-Path $media 'h264_64.mkv')
Assert-Title 'a file that lists no frames up front says so' '^eyepiece . .*h264_64\.mkv: this kind of file does not list its frames up front'
Stop-Player
Start-Player (Join-Path $media 'no such clip.mov')
Assert-Title 'a missing file says so' '^eyepiece . .*no such clip\.mov'
Stop-Player

if ($script:failed) { Write-Host "$($script:failed) check(s) failed"; exit 1 }
Write-Host 'all checks hold'
