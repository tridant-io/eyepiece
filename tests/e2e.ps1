# end to end: starts the real program on tiny clips, drives it with real window messages and checks what it says.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\e2e.ps1 [-Exe path\to\eyepiece.exe]
# windows open for a few seconds each. exits 0 when every check holds. needs a desktop session and a gpu with a
# video decoder (the h.264 and hevc checks require hardware decoding).
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
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
'@
# the controls are driven by posted input, not the real pointer and keyboard: the player is told, so it does not ask
# windows where the real pointer is, and takes what is held with a key (1 shift, 2 alt, 4 control) from the message
$env:EYEPIECE_POSTED_POINTER = '1'
$script:failed = 0
# the pointer, in the design's pixels. a negative place counts from the right or the bottom edge of the picture area
function Get-Place([double]$x, [double]$y) {
    $c = New-Object EyepieceE2e+RECT; [void][EyepieceE2e]::GetClientRect($script:h, [ref]$c)
    $scale = [EyepieceE2e]::GetDpiForWindow($script:h) / 96.0
    $px = if ($x -lt 0) { $c.R + $x * $scale } else { $x * $scale }
    $py = if ($y -lt 0) { $c.B + $y * $scale } else { $y * $scale }
    [IntPtr][int64](([int]$py -shl 16) -bor ([int]$px -band 0xffff))
}
function Move-Pointer([double]$x, [double]$y) { [void][EyepieceE2e]::PostMessage($script:h, 0x200, [IntPtr]0, (Get-Place $x $y)); Start-Sleep -Milliseconds 350 }
# a click: the pointer goes there, the controls come up, the button goes down and up. -Again clicks where the pointer
# already is, with no wait
function Click-At([double]$x, [double]$y, [switch]$Again) {
    $at = Get-Place $x $y
    if (-not $Again) { [void][EyepieceE2e]::PostMessage($script:h, 0x200, [IntPtr]0, $at); Start-Sleep -Milliseconds 350 }
    [void][EyepieceE2e]::PostMessage($script:h, 0x201, [IntPtr]1, $at); Start-Sleep -Milliseconds 60
    [void][EyepieceE2e]::PostMessage($script:h, 0x202, [IntPtr]0, $at)
    if (-not $Again) { Start-Sleep -Milliseconds 200 }
}
# two quick clicks in one place
function DoubleClick-At([double]$x, [double]$y) {
    $at = Get-Place $x $y
    [void][EyepieceE2e]::PostMessage($script:h, 0x200, [IntPtr]0, $at); Start-Sleep -Milliseconds 350
    foreach ($i in 1, 2) { [void][EyepieceE2e]::PostMessage($script:h, 0x201, [IntPtr]1, $at); Start-Sleep -Milliseconds 40; [void][EyepieceE2e]::PostMessage($script:h, 0x202, [IntPtr]0, $at); Start-Sleep -Milliseconds 40 }
    Start-Sleep -Milliseconds 200
}
function Get-Height { $c = New-Object EyepieceE2e+RECT; [void][EyepieceE2e]::GetClientRect($script:h, [ref]$c); $c.B / ([EyepieceE2e]::GetDpiForWindow($script:h) / 96.0) }
function Get-Width { $c = New-Object EyepieceE2e+RECT; [void][EyepieceE2e]::GetClientRect($script:h, [ref]$c); $c.R / ([EyepieceE2e]::GetDpiForWindow($script:h) / 96.0) }
function Get-Title { $s = New-Object Text.StringBuilder 512; [void][EyepieceE2e]::GetWindowText($script:h, $s, 512); $s.ToString() }
function Send-Key([int]$vk, [int]$times = 1, [int]$with = 0) { foreach ($i in 1..$times) { [void][EyepieceE2e]::PostMessage($script:h, 0x100, [IntPtr]$vk, [IntPtr]$with); Start-Sleep -Milliseconds 200 } }
function Send-Char([string]$text) { foreach ($c in $text.ToCharArray()) { [void][EyepieceE2e]::PostMessage($script:h, 0x102, [IntPtr][int]$c, [IntPtr]0); Start-Sleep -Milliseconds 80 } }
function Confirm-That([string]$what, [bool]$holds, [string]$detail = '') {
    if ($holds) { Write-Host "ok    $what" } else { Write-Host "FAIL  ${what}: $detail"; $script:failed++ }
}
# the title is set by another thread: it must come to satisfy the test within five seconds (a busy machine is slow)
function Wait-Title([string]$what, [scriptblock]$test, [string]$wanted) {
    foreach ($i in 1..50) { if (& $test (Get-Title)) { Write-Host "ok    $what"; return }; Start-Sleep -Milliseconds 100 }
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
# zoom. the pointer is put in the middle of the window, over the middle of the 64 x 64 picture
Move-Pointer ((Get-Width) / 2) ((Get-Height) / 2)
Send-Key 0x31; Assert-Title '1 is 1:1, and the title names the pixel under the pointer with its values' ' . 100% . x 32 y 32 . \d+ \d+ \d+$'
[void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](120 * 65536 + 4), [IntPtr]0); Assert-Title 'shift + wheel zooms in one stop, about the pointer' ' . 200% . x 32 y 32 . '
Send-Key 0x25 1 2; Assert-Title 'alt + left moves the picture right, as far as its edge' ' . 200% . x 0 y 32 . '
Send-Key 0x30; Assert-Frame '0 fits the picture again' 0 3
DoubleClick-At ((Get-Width) / 2) ((Get-Height) / 2); Assert-Title 'a double click goes back to the last zoom and place' ' . 200% . x 0 y 32 . '
DoubleClick-At ((Get-Width) / 2) ((Get-Height) / 2); Assert-Frame 'and another fits it' 0 3
# the zoom button is the number at the right of the bar. its menu stands over it: the choices are 181 up from the
# bottom, 1:1 the second of five; the field is 143 up
Click-At -30 -32; Click-At -188 -181; Assert-Title 'the zoom menu''s 1:1 choice' ' . 100%'
Click-At -50 -143; Send-Key 0x41 1 4; Send-Char '250'; Send-Key 0x0D; Assert-Title 'a zoom typed into the menu''s field' ' . 250%'
Send-Key 0x1B; Send-Key 0x30; Assert-Frame 'escape closes the menu, and 0 fits the picture' 0 3
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

# ---- the controls themselves, with a pointer: the bar's buttons, the timebar, the strip's close button.
# places are from the design: the row's buttons are 30 wide from 8, 32 up from the bottom; the timebar is the bottom edge
Send-Key 0x24; Assert-Frame 'back to the first frame for the pointer checks' 0 24
Click-At 87 -32; Assert-Frame 'the next-frame button steps forward' 1 24
Click-At 87 -32; Assert-Frame 'and again' 2 24
Click-At 23 -32; Assert-Frame 'the previous-frame button steps back' 1 24
# a quarter of the way along 24 frames is frame 6 (5.75, to the nearest frame)
Click-At ((Get-Width) * 0.25) -3; Assert-Frame 'a click on the timebar goes to that frame' 6 24
Click-At ((Get-Width) - 1) -3; Assert-Frame 'its right end is the last frame' 23 24
Click-At 0 -3; Assert-Frame 'its left end is the first' 0 24
# the play button: the clip is 24 frames and 0.8 s long. a third of a second of play ends inside it, past frame 0,
# however busy the machine is
Click-At 55 -32; Start-Sleep -Milliseconds 80; Click-At 55 -32 -Again
Wait-Title 'the play button plays, and pauses' { param($t) $t -match ' (\d+) / 24 ' -and [int]$Matches[1] -ge 1 } 'a frame past 0'
# the strip's own window buttons, from the right: close, maximize, minimize, 40 wide each
Click-At -60 16; Start-Sleep -Milliseconds 400
Confirm-That 'the strip''s maximize button maximizes the window' ([EyepieceE2e]::IsZoomed($script:h)) 'the window is not maximized'
Send-Key 0x24; Assert-Frame 'maximized, it still answers' 0 24
Click-At -60 16; Start-Sleep -Milliseconds 400
Confirm-That 'and puts it back' (-not [EyepieceE2e]::IsZoomed($script:h)) 'the window is still maximized'
Click-At -100 16; Start-Sleep -Milliseconds 400
Confirm-That 'the strip''s minimize button minimizes the window' ([EyepieceE2e]::IsIconic($script:h)) 'the window is not minimized'
[void][EyepieceE2e]::ShowWindow($script:h, 9); Start-Sleep -Milliseconds 400
Send-Key 0x47; Assert-Frame 'brought back, it still answers' 1 24
# the close mark at the right end of the strip closes the window
Move-Pointer 100 100
Click-At -20 16
Confirm-That 'the strip''s close button closes the window' ($script:p.WaitForExit(3000) -and $script:p.ExitCode -eq 0) 'still running, or a bad exit code'
if (-not $script:p.HasExited) { $script:p.Kill() }

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
