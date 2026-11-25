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
  [DllImport("user32.dll")] public static extern IntPtr GetClassLongPtrW(IntPtr h, int index);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int index);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
'@
# what the window shows, asked of the window itself (PrintWindow: its own picture, not the screen's)
Add-Type -ReferencedAssemblies System.Drawing @'
using System; using System.Drawing; using System.Runtime.InteropServices;
public static class EyepiecePicture {
  [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out Rect r);
  [StructLayout(LayoutKind.Sequential)] struct Rect { public int L, T, R, B; }
  // pixels of bright text (every channel over 200) in a part of the window's picture area; with `any`, pixels that are
  // not near black (a channel over 60)
  public static int Bright(IntPtr h, int x0, int y0, int x1, int y1, bool any) {
    Rect c; GetClientRect(h, out c);
    using (var bmp = new Bitmap(c.R, c.B)) {
      using (var g = Graphics.FromImage(bmp)) { var dc = g.GetHdc(); PrintWindow(h, dc, 3); g.ReleaseHdc(dc); }
      int n = 0;
      for (int y = Math.Max(0, y0); y < Math.Min(c.B, y1); y++)
        for (int x = Math.Max(0, x0); x < Math.Min(c.R, x1); x++) { var p = bmp.GetPixel(x, y); if (any ? Math.Max(p.R, Math.Max(p.G, p.B)) > 60 : Math.Min(p.R, Math.Min(p.G, p.B)) > 200) n++; }
      return n;
    }
  }
  // the first row from the top with a pixel of bright text between x0 and x1, or -1
  public static int FirstBright(IntPtr h, int x0, int y0, int x1, int y1) {
    Rect c; GetClientRect(h, out c);
    using (var bmp = new Bitmap(c.R, c.B)) {
      using (var g = Graphics.FromImage(bmp)) { var dc = g.GetHdc(); PrintWindow(h, dc, 3); g.ReleaseHdc(dc); }
      for (int y = Math.Max(0, y0); y < Math.Min(c.B, y1); y++)
        for (int x = Math.Max(0, x0); x < Math.Min(c.R, x1); x++) { var p = bmp.GetPixel(x, y); if (Math.Min(p.R, Math.Min(p.G, p.B)) > 200) return y; }
      return -1;
    }
  }
}
'@
# the controls are driven by posted input, not the real pointer and keyboard: the player is told, so it listens only
# to pointer messages that carry the tests' mark (0x8000) and to keys with no scan code, and takes what is held with
# a key (1 shift, 2 alt, 4 control) from the message. a person using the machine meanwhile does not disturb it
$env:EYEPIECE_POSTED_POINTER = '1'
# the player keeps its settings under APPDATA. the tests give it a folder of their own, so every run starts from the
# defaults and the owner's own settings are left alone
$env:APPDATA = Join-Path $env:TEMP "eyepiece-e2e-$PID"
# and muted, so nothing the tests play is heard. the browser starts in that folder too: a new tab then shows nothing
# of the owner's, and makes no pictures of it
New-Item -ItemType Directory -Force (Join-Path $env:APPDATA 'eyepiece') | Out-Null
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value 'mute = on', "browser_folder = $env:APPDATA"
# the browser's pictures are kept in the os cache folder. the tests name a folder of their own to stand for it, so
# nothing they make lands in the owner's cache
$env:EYEPIECE_CACHE = Join-Path $env:APPDATA 'os-cache'
$previews = Join-Path $env:EYEPIECE_CACHE 'eyepiece\cache\previews'
# one player per user: a launch hands its file to a player already running. players started with the same
# EYEPIECE_INSTANCE find each other and no other, so these never hand a file to one of the owner's
$env:EYEPIECE_INSTANCE = "e2e-$PID"
$script:failed = 0
# the pointer, in the design's pixels. a negative place counts from the right or the bottom edge of the picture area
function Get-Place([double]$x, [double]$y) {
    $c = New-Object EyepieceE2e+RECT; [void][EyepieceE2e]::GetClientRect($script:h, [ref]$c)
    $scale = [EyepieceE2e]::GetDpiForWindow($script:h) / 96.0
    $px = if ($x -lt 0) { $c.R + $x * $scale } else { $x * $scale }
    $py = if ($y -lt 0) { $c.B + $y * $scale } else { $y * $scale }
    [IntPtr][int64](([int]$py -shl 16) -bor ([int]$px -band 0xffff))
}
function Move-Pointer([double]$x, [double]$y) { [void][EyepieceE2e]::PostMessage($script:h, 0x200, [IntPtr]0x8000, (Get-Place $x $y)); Start-Sleep -Milliseconds 350 }
# a click: the pointer goes there, the controls come up, the button goes down and up. -Again clicks where the pointer
# already is, with no wait
function Click-At([double]$x, [double]$y, [switch]$Again) {
    $at = Get-Place $x $y
    if (-not $Again) { [void][EyepieceE2e]::PostMessage($script:h, 0x200, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 350 }
    [void][EyepieceE2e]::PostMessage($script:h, 0x201, [IntPtr]0x8001, $at); Start-Sleep -Milliseconds 60
    [void][EyepieceE2e]::PostMessage($script:h, 0x202, [IntPtr]0x8000, $at)
    if (-not $Again) { Start-Sleep -Milliseconds 200 }
}
# two quick clicks in one place
function DoubleClick-At([double]$x, [double]$y) {
    $at = Get-Place $x $y
    [void][EyepieceE2e]::PostMessage($script:h, 0x200, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 350
    foreach ($i in 1, 2) { [void][EyepieceE2e]::PostMessage($script:h, 0x201, [IntPtr]0x8001, $at); Start-Sleep -Milliseconds 40; [void][EyepieceE2e]::PostMessage($script:h, 0x202, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 40 }
    Start-Sleep -Milliseconds 200
}
# a right click: the pointer goes there, the right button goes down and up
function RightClick-At([double]$x, [double]$y) {
    $at = Get-Place $x $y
    [void][EyepieceE2e]::PostMessage($script:h, 0x200, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 350
    [void][EyepieceE2e]::PostMessage($script:h, 0x204, [IntPtr]0x8002, $at); Start-Sleep -Milliseconds 60
    [void][EyepieceE2e]::PostMessage($script:h, 0x205, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 200
}
# bright text between two corners of the window, in the design's pixels as Get-Place takes them; with -Any, what is not
# near black
function Get-Bright([double]$x0, [double]$y0, [double]$x1, [double]$y1, [switch]$Any) {
    $a = (Get-Place $x0 $y0).ToInt64(); $b = (Get-Place $x1 $y1).ToInt64()
    [EyepiecePicture]::Bright($script:h, [int]($a -band 0xffff), [int]($a -shr 16), [int]($b -band 0xffff), [int]($b -shr 16), $Any.IsPresent)
}
# the first line of bright text from the top between two corners, in the design's pixels, or -1
function Get-FirstBright([double]$x0, [double]$y0, [double]$x1, [double]$y1) {
    $a = (Get-Place $x0 $y0).ToInt64(); $b = (Get-Place $x1 $y1).ToInt64()
    $y = [EyepiecePicture]::FirstBright($script:h, [int]($a -band 0xffff), [int]($a -shr 16), [int]($b -band 0xffff), [int]($b -shr 16))
    if ($y -lt 0) { -1 } else { $y / ([EyepieceE2e]::GetDpiForWindow($script:h) / 96.0) }
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
    # with no clip the player starts on the browser
    $more = @{}; $script:name = 'eyepiece'
    if ($clip) { $script:name = Split-Path $clip -Leaf; $more.ArgumentList = "`"$clip`"" }
    $script:p = Start-Process $Exe @more -PassThru
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
# true while the window has its frame: false in fullscreen
function Test-Framed { ([EyepieceE2e]::GetWindowLong($script:h, -16) -band 0x40000) -ne 0 }
# the middle of the picture area
function Get-Middle { ((Get-Width) / 2), ((Get-Height) / 2) }

$media = Join-Path $PSScriptRoot 'media'

# ---- hap: our own lane, and everything about the window
Start-Player (Join-Path $media 'hapq_64.mov')
Assert-Title 'hap q opens on frame 0 and reads the file exactly' '^hapq_64\.mov . 0 / 3 . 64 . 64 . hap q . 60000/1001 fps$'
# the window's class carries the icon linked into the exe (-14 is GCLP_HICON): what the taskbar shows for it
Confirm-That 'the window carries the program''s icon' ([EyepieceE2e]::GetClassLongPtrW($script:h, -14) -ne [IntPtr]::Zero) 'the window''s class has no icon'
# round the picture area, what is seen of the window (9 is DWMWA_EXTENDED_FRAME_BOUNDS) is the system's line and
# no more, the same on every side: the program asks for it in black, so nothing of the window is bare. with the
# line on three sides only and no color asked for, a window behind showed through a pixel's width of this one
$seen = New-Object EyepieceE2e+RECT; $area = New-Object EyepieceE2e+RECT; $corner = New-Object EyepieceE2e+POINT
[void][EyepieceE2e]::DwmGetWindowAttribute($script:h, 9, [ref]$seen, 16); [void][EyepieceE2e]::GetClientRect($script:h, [ref]$area); [void][EyepieceE2e]::ClientToScreen($script:h, [ref]$corner)
$line = ($corner.X - $seen.L), ($corner.Y - $seen.T), ($seen.R - $corner.X - $area.R), ($seen.B - $corner.Y - $area.B)
Confirm-That 'the line round the picture area is the same on every side' (($line | Select-Object -Unique).Count -eq 1 -and $line[0] -le 2) "at the left, top, right, bottom: $($line -join ', ') px"
Send-Key 0x47; Assert-Frame 'G steps forward' 1 3
Send-Key 0x27; Assert-Frame 'right arrow steps forward' 2 3
Send-Key 0x47; Assert-Frame 'a step past the end stays on the last frame' 2 3
Send-Key 0x46; Assert-Frame 'F steps back' 1 3
Send-Key 0x24; Assert-Frame 'home goes to the first frame' 0 3
Send-Key 0x23; Assert-Frame 'end goes to the last frame' 2 3
# the wheel: one notch toward the user is one frame forward, away is one back
Send-Key 0x24
[void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](-120 * 65536 + 0x8000), [IntPtr]0); Assert-Frame 'a wheel notch toward the user steps forward' 1 3
[void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](120 * 65536 + 0x8000), [IntPtr]0); Assert-Frame 'a wheel notch away steps back' 0 3
# zoom. the pointer is put in the middle of the window, over the middle of the 64 x 64 picture
$mid = Get-Middle
Move-Pointer $mid[0] $mid[1]
Send-Key 0x31; Assert-Title '1 is 1:1, and the title names the pixel under the pointer with its values' ' . 100% . x 32 y 32 . \d+ \d+ \d+$'
[void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](120 * 65536 + 0x8004), [IntPtr]0); Assert-Title 'shift + wheel zooms in one stop, about the pointer' ' . 200% . x 32 y 32 . '
# the navigator: zoomed in past the window (several stops on, to 1200% or more: 64 pixels are then 768 or more), a
# plan of the picture stands at the lower left, 112 square for this one, its foot 62 above the window's while the bar
# is up. a press near its upper left corner brings that corner of the picture as near the middle as it goes: at
# 1200% the window shows 53 by 30 of the picture's pixels, so the middle is then at 26, 15
foreach ($i in 1..6) { Send-Key 0xBB }
Move-Pointer $mid[0] $mid[1]; Assert-Title 'zoomed far in, the middle of the picture stays under the middle of the window' ' . 1[26]00% . x 32 y 32 . '
Click-At 16 -168; Move-Pointer $mid[0] $mid[1]
Wait-Title 'a press in the navigator brings that part of the picture to the middle' { param($t) $t -match ' x (\d+) y (\d+) ' -and [int]$Matches[1] -le 27 -and [int]$Matches[2] -le 15 } 'x 26 y 15 or less'
Send-Key 0x30; Assert-Frame '0 fits the picture again' 0 3
# the bar, from the right: fullscreen, sound, screenshots, metadata and the view (30 wide each), then the zoom, the speed
# and the loop by name.
# a menu stands over its own button, its right edge 4 past the button's. the zoom menu's choices are 181 up from the
# bottom, 1:1 the second of five; its field is 143 up
Click-At -203 -32; Click-At -346 -181; Assert-Title 'the zoom menu''s 1:1 choice' ' . 100%'
Click-At -219 -143; Send-Key 0x41 1 4; Send-Char '250'; Send-Key 0x0D; Assert-Title 'a zoom typed into the menu''s field' ' . 250%'
Click-At -219 -143; Send-Key 0x41 1 4; Send-Char '1600'; Send-Key 0x0D; Send-Key 0x1B
Move-Pointer $mid[0] $mid[1]; Assert-Title 'escape closes the menu; at 1600% the middle of the picture is still under the middle of the window' ' . 1600% . x 32 y 32 . '
Send-Key 0x25 1 2; Assert-Title 'alt + left moves the picture right by an eighth of the window' ' . 1600% . x 2\d y 32 . '
Send-Key 0x30; Assert-Frame '0 fits the picture' 0 3
# a digit the player has no key for (3), typed by its key right after a key that types nothing (home), reaches the
# field: the character of one press is never taken for the command before it. the field holds the fit's 563, and
# what is typed goes in before it
Send-Key 0x24; Click-At -203 -32; Click-At -219 -143; Send-Key 0x33; Send-Key 0x0D
Assert-Title 'a digit typed after a key that types nothing reaches the zoom field' ' . 3563%'
Send-Key 0x1B; Send-Key 0x30; Move-Pointer $mid[0] $mid[1]; Assert-Frame '0 fits the picture again' 0 3

# speed, loop and loop analysis, by key
# - and = zoom; with shift they step the speed
Send-Key 0xBB; Assert-Title '= zooms in one stop: from fit, 562.5%, to 600%' ' . 600% . x 32 y 32 . '
Send-Key 0xBD; Assert-Frame '- zooms out one stop, back to fit' 0 3
Send-Key 0xBB 1 1; Assert-Title 'shift + = is one stop faster' ' . 1\.5.$'
Send-Key 0x08; Assert-Frame 'backspace is 1x again' 0 3
Send-Key 0x52; Assert-Title 'R turns looping off' ' . loop off$'
Send-Key 0x52; Assert-Frame 'and on again' 0 3
# three frames: the round is the last frame, the seam, the first frame
Send-Key 0x41; Assert-Title 'A starts loop analysis on the frame before the seam' '^hapq_64\.mov . 2 / 3 . .* . loop analysis$'
Send-Key 0x47; Assert-Title 'a step crosses the seam to the first frame' '^hapq_64\.mov . 0 / 3 . .* . loop analysis$'
Send-Key 0x41; Assert-Frame 'A leaves loop analysis' 0 3
# and by pointer: the speed menu's 2x is the fourth of five choices, 122 up; the loop menu's off is the first of two
Click-At -256 -32; Click-At -323 -122; Assert-Title 'the speed menu''s 2x choice' ' . 2.$'
Send-Key 0x1B; Send-Key 0x08; Assert-Frame 'backspace is 1x again, with the menu closed' 0 3
Click-At -302 -32; Click-At -463 -181; Assert-Title 'the loop menu''s off choice' ' . loop off$'
Click-At -511 -143; Assert-Title 'the loop menu''s switch starts loop analysis' ' . loop analysis$'
Click-At -511 -143; Click-At -343 -181; Send-Key 0x1B; Send-Key 0x24; Assert-Frame 'the switch again, then the one choice: looping as before' 0 3

# the view: guides, the look, one channel
Send-Key 0x44; Assert-Title 'D shows the grid' ' . grid$'
Send-Key 0x43; Assert-Title 'C shows the center lines and circle' ' . grid . center$'
Send-Key 0x44; Send-Key 0x43; Assert-Frame 'and both go away' 0 3
Send-Key 0x48; Move-Pointer $mid[0] $mid[1]; Send-Key 0x31
Assert-Title 'H mirrors: the pixel named is the file''s, from its other side' ' . mirrored . 100% . x 31 y 32 . '
Send-Key 0x48; Assert-Frame 'H again shows the picture as the file has it, fitted' 0 3
Send-Key 0x51; Assert-Title 'Q turns it a quarter' ' . rotated 90.$'
Send-Key 0x56; Assert-Title 'V flips it' ' . flipped . rotated 90.$'
Send-Key 0x56; Send-Key 0x51 3; Assert-Frame 'three more quarters and it is upright' 0 3
# the view menu: the channels are 126 up from the bottom, r the second of five; mirror is the first button, 86 up
Click-At -151 -32; Click-At -314 -126; Assert-Title 'the view menu''s r choice shows red alone' ' . red only$'
Click-At -362 -126; Click-At -347 -86; Assert-Title 'rgb again, and the mirror button' ' . mirrored$'
Click-At -347 -86; Send-Key 0x1B; Assert-Frame 'the mirror button again' 0 3

# the keys (fig. 21). a button's tooltip names what it does and its key, a moment after the pointer rests on it, in a
# chip over the bar: for the play button from its left edge, in the black beside the picture
$tip = 48, -90, 130, -58
Move-Pointer 300 150; Start-Sleep -Milliseconds 900
$before = Get-Bright @tip
Move-Pointer 95 -32; Start-Sleep -Milliseconds 900
Confirm-That 'resting on the play button brings its tooltip' ($before -eq 0 -and (Get-Bright @tip) -gt 0) "bright pixels: $before before, $(Get-Bright @tip) after"
Click-At -256 -32; Move-Pointer 95 -32; Start-Sleep -Milliseconds 900
Confirm-That 'no tooltip while a menu is open' ((Get-Bright @tip) -eq 0) "bright pixels: $(Get-Bright @tip)"
Send-Key 0x1B
# ? (shift + slash) lays the table of keys over the clip. a click takes it away and does nothing else: a double click
# on the picture over it does not fill the display
Send-Key 0xBF 1 1; DoubleClick-At $mid[0] $mid[1]; Start-Sleep -Milliseconds 500
Confirm-That 'a double click over the table of keys only takes it away' (Test-Framed) "framed: $(Test-Framed)"
# the table is bright keys in the black beside the picture
$band = 20, 60, 130, 250
Move-Pointer 300 150; Start-Sleep -Milliseconds 300
$before = Get-Bright @band
Send-Key 0xBF 1 1
Confirm-That '? shows the table of keys' ($before -eq 0 -and (Get-Bright @band) -gt 0) "bright pixels: $before before, $(Get-Bright @band) with it"
Send-Key 0x1B
Confirm-That 'esc takes it away' ((Get-Bright @band) -eq 0) "bright pixels: $(Get-Bright @band)"
Assert-Frame 'and leaves the player as it was' 0 3
# esc takes the table away before it leaves fullscreen
Send-Key 0x50; Start-Sleep -Milliseconds 500; Send-Key 0xBF 1 1; Send-Key 0x1B; Start-Sleep -Milliseconds 300
Confirm-That 'esc over the table in fullscreen takes the table away and stays fullscreen' (-not (Test-Framed)) "framed: $(Test-Framed)"
Send-Key 0x1B; Start-Sleep -Milliseconds 500
Confirm-That 'and the next esc leaves fullscreen' (Test-Framed) "framed: $(Test-Framed)"
# a right click on the picture opens every command, at the pointer; choosing a row does what its key does. the menu
# is too tall for any window here and scrolls: in a 900 x 700 window, at the end of its list, the mirror row's middle
# is 435 up from the bottom (24-high rows, sections 10 apart, the menu 8 in from the window's edges)
[void][EyepieceE2e]::SetWindowPos($script:h, [IntPtr]::Zero, 0, 0, 900, 700, 0x0006); Start-Sleep -Milliseconds 300
RightClick-At 120 100
Confirm-That 'a right click opens the menu and chooses nothing' (Test-Framed) "framed: $(Test-Framed)"
Assert-Frame 'and the picture is as it was' 0 3
Move-Pointer 200 200
foreach ($i in 1..40) { [void][EyepieceE2e]::PostMessage($script:h, 0x20A, [IntPtr](-120 * 65536 + 0x8000), [IntPtr]0); Start-Sleep -Milliseconds 20 }
Start-Sleep -Milliseconds 1000
Assert-Frame 'the wheel over the menu turns its list and does not scrub' 0 3
Click-At 200 -435; Click-At 200 -435 -Again; Start-Sleep -Milliseconds 500
Assert-Title 'its mirror row mirrors the picture' ' . mirrored$'
Confirm-That 'and a click at once on the picture under it makes no double click: the window keeps its frame' (Test-Framed) "framed: $(Test-Framed)"
Send-Key 0x48; Assert-Frame 'H shows it as the file has it again' 0 3
# the click after esc is one on the picture, which plays; a second pauses, and home goes back
RightClick-At 120 100; Send-Key 0x1B; Click-At 200 -435; Click-At 200 -435
Confirm-That 'esc closes the menu: a click where the row was chooses nothing' ((Get-Title) -notmatch 'mirrored') "the title is '$(Get-Title)'"
Send-Key 0x24; Assert-Frame 'and home is the first frame again' 0 3

# fullscreen: the frame goes and the picture area is a whole display; escape puts the window back
$before = Get-Width
# with the pointer on the bar, so the controls are up as it begins
Move-Pointer 200 -32; Start-Sleep -Milliseconds 300
Send-Key 0x50; Start-Sleep -Milliseconds 500
Confirm-That 'P fills the display' ((-not (Test-Framed)) -and (Get-Width) -gt $before) "framed: $(Test-Framed), width $(Get-Width)"
Confirm-That 'and takes the controls away as it does: the bar''s row is bare' ((Get-Bright 40 -42 150 -22) -eq 0) "bright pixels in the bar's row: $(Get-Bright 40 -42 150 -22)"
Send-Key 0x47; Assert-Frame 'it still steps in fullscreen' 1 3
Send-Key 0x1B; Start-Sleep -Milliseconds 500
Confirm-That 'escape puts the window back as it was' ((Test-Framed) -and (Get-Width) -eq $before) "framed: $(Test-Framed), width $(Get-Width), was $before"
Send-Key 0x24; Assert-Frame 'and it still steps' 0 3
# a double click anywhere on the picture does the same
DoubleClick-At $mid[0] $mid[1]; Start-Sleep -Milliseconds 500
Confirm-That 'a double click on the picture fills the display' (-not (Test-Framed)) "framed: $(Test-Framed)"
$fullMid = Get-Middle
DoubleClick-At $fullMid[0] $fullMid[1]; Start-Sleep -Milliseconds 500
Confirm-That 'and another puts the window back as it was' ((Test-Framed) -and (Get-Width) -eq $before) "framed: $(Test-Framed), width $(Get-Width), was $before"

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
# what is kept between runs: the grid, switched on, is there when the player starts again
Send-Key 0x44; Assert-Title 'the grid is on' ' . grid$'
Stop-Player
Start-Player (Join-Path $media 'hapq_64.mov')
Assert-Title 'the grid is kept between runs' '^hapq_64\.mov . 0 / 3 . .* . grid$'
Send-Key 0x44; Assert-Frame 'and switched off again' 0 3
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
# the timebar's picture (fig. 04) while a clip with long groups plays, when no frame of it is read: the timebar strip's,
# made while the clip stood still. the window is tall, so the picture's place, 82 to 172 up from the bottom, is over
# black and not over the clip
[void][EyepieceE2e]::SetWindowPos($script:h, [IntPtr]::Zero, 0, 0, 640, 900, 0x0006); Start-Sleep -Milliseconds 300
$picture = 250, -165, 390, -90
Move-Pointer 320 300; Start-Sleep -Milliseconds 300
$before = Get-Bright @picture -Any
Send-Key 0x20; Move-Pointer 320 -5; $during = Get-Bright @picture -Any; Send-Key 0x20
Confirm-That 'while a clip with long groups plays, the timebar shows a picture of its timebar strip' ($before -eq 0 -and $during -gt 2000) "lit pixels: $before before, $during on the timebar"

# ---- the controls themselves, with a pointer: the bar's buttons, the timebar, the strip's close button.
# places are from the design: the row's buttons are 30 wide, 32 up from the bottom: the pin from 8, then previous
# frame from 48, play and next frame; the timebar is the bottom edge
Send-Key 0x24; Assert-Frame 'back to the first frame for the pointer checks' 0 24
Click-At 127 -32; Assert-Frame 'the next-frame button steps forward' 1 24
Click-At 127 -32; Assert-Frame 'and again' 2 24
Click-At 63 -32; Assert-Frame 'the previous-frame button steps back' 1 24
# a quarter of the way along 24 frames is frame 6 (5.75, to the nearest frame)
Click-At ((Get-Width) * 0.25) -3; Assert-Frame 'a click on the timebar goes to that frame' 6 24
Click-At ((Get-Width) - 1) -3; Assert-Frame 'its right end is the last frame' 23 24
Click-At 0 -3; Assert-Frame 'its left end is the first' 0 24
# the play button: the clip is 24 frames and 0.8 s long. a third of a second of play ends inside it, past frame 0,
# however busy the machine is
Click-At 95 -32; Start-Sleep -Milliseconds 80; Click-At 95 -32 -Again
Wait-Title 'the play button plays, and pauses' { param($t) $t -match ' (\d+) / 24 ' -and [int]$Matches[1] -ge 1 } 'a frame past 0'
# a plain click on the picture plays, and another pauses (the owner's word): two clicks too far apart to be a double
# click, a third of a second of play between them
$m = Get-Middle
Send-Key 0x24; Assert-Frame 'home, for the click on the picture' 0 24
Click-At $m[0] $m[1]; Click-At $m[0] $m[1]
Wait-Title 'a click on the picture plays, and another pauses' { param($t) $t -match ' (\d+) / 24 ' -and [int]$Matches[1] -ge 1 } 'a frame past 0'
Confirm-That 'and two clicks apart are no double click: the window keeps its frame' (Test-Framed) "framed: $(Test-Framed)"
# the pin at the bar's far left keeps the controls up: the play button's mark is still drawn three seconds after
# the pointer came to rest on the picture, where without the pin the controls go after 2.4
$mark = 84, -42, 106, -22
Click-At 23 -32; Move-Pointer $m[0] $m[1]; Start-Sleep -Milliseconds 3200
Confirm-That 'pinned, the controls stay up while the pointer rests' ((Get-Bright @mark) -gt 0) "bright pixels of the play button: $(Get-Bright @mark)"
Click-At 23 -32; Move-Pointer $m[0] $m[1]; Start-Sleep -Milliseconds 3200
Confirm-That 'unpinned, they go again' ((Get-Bright @mark) -eq 0) "bright pixels of the play button: $(Get-Bright @mark)"
# shuttle: L runs forward and K stops; J runs backward. a third of a second ends inside the clip, off frame 0
Send-Key 0x24; Send-Key 0x4C; Start-Sleep -Milliseconds 100; Send-Key 0x4B
Wait-Title 'L plays forward, K stops' { param($t) $t -match ' (\d+) / 24 ' -and [int]$Matches[1] -ge 1 } 'a frame past 0'
Send-Key 0x24; Send-Key 0x4A; Start-Sleep -Milliseconds 100; Send-Key 0x4B
Wait-Title 'J plays backward, K stops' { param($t) $t -match ' (\d+) / 24 ' -and [int]$Matches[1] -ge 1 } 'a frame before the first, which is one near the end'
# with looping off, play stops on the last frame
Send-Key 0x24; Send-Key 0x52; Send-Key 0x20
Assert-Title 'with looping off, play stops by itself on the last frame' '^h264_320\.mp4 . 23 / 24 . .* . loop off$'
Send-Key 0x52; Send-Key 0x24; Assert-Frame 'looping on again' 0 24
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

# ---- tabs: a clip, a new tab with the browser, round them, the browser over a clip, close and bring back
Start-Player (Join-Path $media 'h264_320.mp4')
Send-Key 0x47; Assert-Frame 'the clip is on frame 1' 1 24
Send-Key 0x4E; Assert-Title 'N opens a new tab: the browser, and no clip' '^eyepiece$'
Send-Key 0xDB; Assert-Frame '[ goes round to the clip, which is where it was' 1 24
Send-Key 0x4F; Assert-Title 'O lays the browser over the clip' ' . browser$'
Send-Key 0x1B; Assert-Frame 'escape takes it away again' 1 24
Send-Key 0xDD; Assert-Title '] goes round to the new tab' '^eyepiece$'
Send-Key 0x57 1 4; Assert-Frame 'control + W closes the tab that is up' 1 24
Send-Key 0x57 1 4; Assert-Title 'closing the last tab leaves an empty one, and the window stays' '^eyepiece$'
Send-Key 0x54 1 5; Assert-Frame 'control + shift + T brings the closed tab back on its frame' 1 24
# the strip: the clip's tab is as wide as its name and 40 more (123 here), then the + that opens a new tab
Click-At 139 16; Assert-Title 'the + opens a new tab' '^eyepiece$'
Click-At 60 16; Assert-Frame 'a click on a tab puts it up' 1 24
Click-At 109 16; Assert-Title 'the mark on the tab that is up closes it' '^eyepiece$'
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

# ---- stills: a numbered run is one clip, opened on the frame of the file given; a frame with no file says so
Start-Player (Join-Path $media 'seq.0004.png')
Assert-Title 'one file of a numbered run opens the whole run, on its own frame' '^seq\.\[0001-0005\]\.png . 3 / 5 . 16 . 16 . png . 24 fps assumed$'
Send-Key 0x46; Assert-Title 'the frame with no file says so' ' . this frame cannot be shown: this frame.s file is missing$'
Send-Key 0x46; Assert-Title 'and the frames around it show' '^seq\.\[0001-0005\]\.png . 1 / 5 . .* . 1 could not be shown$'
Stop-Player
Start-Player (Join-Path $media 'still_10bit.dpx')
$mid = Get-Middle
Move-Pointer $mid[0] $mid[1]; Send-Key 0x31
Assert-Title 'a lone still is one frame, and a 10-bit dpx gives its values on the 10-bit scale' '^still_10bit\.dpx . 0 / 1 . 16 . 16 . dpx . still . 100% . x 8 y 8 . 520 512 777$'
Stop-Player
Start-Player (Join-Path $media 'still_half.exr')
Assert-Title 'an exr opens' '^still_half\.exr . 0 / 1 . 16 . 16 . exr . still$'
Stop-Player

# ---- screenshots: S saves the file's own frame, shift + S the view as shown, in the folder the settings name.
# the tests name a folder of their own, so nothing lands in the owner's pictures
$shots = Join-Path $env:APPDATA 'shots'
$settings = Join-Path $env:APPDATA 'eyepiece\settings.txt'
(Get-Content -LiteralPath $settings) -replace '^screenshot_folder *=.*$', "screenshot_folder = $shots" | Set-Content -LiteralPath $settings
function Wait-File([string]$file) { foreach ($i in 1..50) { if (Test-Path -LiteralPath $file) { return $true }; Start-Sleep -Milliseconds 100 }; $false }
# a png's width and height, from its header
function Get-PngSize([string]$file) { $b = [IO.File]::ReadAllBytes($file); "$($b[18] * 256 + $b[19]) x $($b[22] * 256 + $b[23])" }
Start-Player (Join-Path $media 'hapq_64.mov')
Assert-Frame 'the clip is open' 0 3
$frame = Join-Path $shots 'hapq_64.f000000.png'
Send-Key 0x53
Confirm-That 'S saves the file''s own frame, named for the clip and the frame, at the file''s size' ((Wait-File $frame) -and (Get-PngSize $frame) -eq '64 x 64') "no $frame of 64 x 64"
$view = Join-Path $shots 'hapq_64.f000000.view.png'
$window = "$([int](Get-Width)) x $([int](Get-Height))"
Send-Key 0x53 1 1
Confirm-That 'shift + S saves the view as shown, at the window''s size' ((Wait-File $view) -and (Get-PngSize $view) -eq $window) "no $view of $window"
Send-Key 0x53
Confirm-That 'a second S never writes over the first: it gets a name of its own' (Wait-File (Join-Path $shots 'hapq_64.f000000 (2).png')) 'no second file'
Stop-Player
# first_frame = 1 in the settings file numbers frames from 1: the title's, and the name of a saved frame
@((Get-Content -LiteralPath $settings) | Where-Object { $_ -notmatch '^first_frame' }) + 'first_frame = 1' | Set-Content -LiteralPath $settings
Start-Player (Join-Path $media 'hapq_64.mov')
Assert-Frame 'with first_frame = 1 the first frame is frame 1' 1 3
$first = Join-Path $shots 'hapq_64.f000001.png'
Send-Key 0x53
Confirm-That 'and S names the saved frame for it' (Wait-File $first) "no $first"
Send-Key 0x23; Assert-Frame 'and the last frame has the number of the count' 3 3
Stop-Player
(Get-Content -LiteralPath $settings) | Where-Object { $_ -notmatch '^first_frame' } | Set-Content -LiteralPath $settings
# chroma that is smaller than the picture (4:2:0 here, from the hardware decoder) is blended up where the file sites
# it: at an edge from green to yellow the pixel between reads a blend. chroma = nearest in the settings file takes
# the nearest sample instead
Start-Player (Join-Path $media 'h264_320.mp4')
Assert-Frame 'the 4:2:0 clip is open' 0 24
Move-Pointer 215 150; Send-Key 0xBB 6
Assert-Title 'at 1600% a pixel on an edge of color is blended from the chroma samples round it' ' . 1600% . x 107 y 75 . 90 218 0$'
Stop-Player
@((Get-Content -LiteralPath $settings) | Where-Object { $_ -notmatch '^chroma' }) + 'chroma = nearest' | Set-Content -LiteralPath $settings
Start-Player (Join-Path $media 'h264_320.mp4')
Assert-Frame 'the 4:2:0 clip is open again' 0 24
Move-Pointer 215 150; Send-Key 0xBB 6
Assert-Title 'with chroma = nearest the same pixel is the nearest sample''s' ' . 1600% . x 107 y 75 . 0 255 1$'
Stop-Player
(Get-Content -LiteralPath $settings) | Where-Object { $_ -notmatch '^chroma' } | Set-Content -LiteralPath $settings
foreach ($made in $frame, (Join-Path $shots 'hapq_64.f000000 (2).png'), $view, $first, $shots) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}

# ---- markup (fig. 19), and what a saved screenshot carries (figs 18, 20). E turns the pointer into a pen: a drag on
# the picture draws a rectangle on the frame on screen, and control + Z takes it back. the screenshot menu's three
# switches put the marks, the guides and a caption into a saved frame, and are kept between runs
Set-Content -LiteralPath $settings -Value 'mute = on', "browser_folder = $env:APPDATA", "screenshot_folder = $shots"
# a drag: the button goes down at one place, the pointer moves there with it held, and it comes up at the other
function Drag-On([double]$x0, [double]$y0, [double]$x1, [double]$y1) {
    Move-Pointer $x0 $y0
    [void][EyepieceE2e]::PostMessage($script:h, 0x201, [IntPtr]0x8001, (Get-Place $x0 $y0)); Start-Sleep -Milliseconds 100
    foreach ($t in 0.25, 0.5, 0.75, 1) { [void][EyepieceE2e]::PostMessage($script:h, 0x200, [IntPtr]0x8001, (Get-Place ($x0 + ($x1 - $x0) * $t) ($y0 + ($y1 - $y0) * $t))); Start-Sleep -Milliseconds 100 }
    [void][EyepieceE2e]::PostMessage($script:h, 0x202, [IntPtr]0x8000, (Get-Place $x1 $y1)); Start-Sleep -Milliseconds 300
}
# a png's color type, from its header: 2 is red, green and blue; 6 is with alpha
function Get-PngColor([string]$file) { [IO.File]::ReadAllBytes($file)[25] }
Start-Player (Join-Path $media 'h264_320.mp4')
Assert-Frame 'the clip is open' 0 24
Send-Key 0x45; Assert-Title 'E turns markup mode on, and the title says so' ' fps . markup$'
# the picture fills the window here; the drag is in its middle, where the pointer does not bring the controls up.
# (that the window stays where it is cannot be told here: posted input never holds the real button, which a move of
# the window needs. the headless test of the controls tells a drag that draws from one that moves the window)
Drag-On 200 120 300 200
Assert-Title 'a drag on the picture draws a mark on the frame' ' fps . 1 mark . markup$'
Send-Key 0x5A 1 4; Assert-Title 'control + Z takes it back' ' fps . markup$'
Drag-On 200 120 300 200; Assert-Title 'a mark again' ' fps . 1 mark . markup$'
Send-Key 0x47; Assert-Title 'a mark sits on its frame: the next one has none' '^h264_320\.mp4 . 1 / 24 .* fps . markup$'
Send-Key 0x46; Assert-Title 'and back on its frame, there it is' '^h264_320\.mp4 . 0 / 24 .* fps . 1 mark . markup$'
Send-Key 0x4E; Send-Key 0xDB; Assert-Title 'it stays while another tab is up' '^h264_320\.mp4 . 0 / 24 .* fps . 1 mark . markup$'
# markup is on in the screenshot menu by default: S saves the frame with the mark. the menu stands over the
# screenshot button (87 from the right); its switches, from the bottom: markup 120 up, guides 146, the caption 173
$marked = Join-Path $shots 'h264_320.f000000.png'
Send-Key 0x53
Confirm-That 'S saves the frame at its own size, opaque' ((Wait-File $marked) -and (Get-PngSize $marked) -eq '320 x 180' -and (Get-PngColor $marked) -eq 2) "no $marked of 320 x 180 without alpha"
Click-At -87 -32; Click-At -250 -120; Send-Key 0x1B
$plain = Join-Path $shots 'h264_320.f000000 (2).png'
Send-Key 0x53
Confirm-That 'with the markup switch off, the saved frame is another picture' ((Wait-File $plain) -and (Get-FileHash -LiteralPath $plain).Hash -ne (Get-FileHash -LiteralPath $marked).Hash) "no $plain, or the same picture"
Click-At -87 -32; Click-At -250 -173; Send-Key 0x1B
$captioned = Join-Path $shots 'h264_320.f000000 (3).png'
Send-Key 0x53
Confirm-That 'with the caption switch on, the saved frame has the caption under it: as wide, and taller' ((Wait-File $captioned) -and (Get-PngSize $captioned) -match '^320 x (\d+)$' -and [int]$Matches[1] -gt 180) "$captioned is $(if (Test-Path -LiteralPath $captioned) { Get-PngSize $captioned })"
Send-Key 0x45; Assert-Title 'E again ends markup mode, and the mark stays' '^h264_320\.mp4 . 0 / 24 .* fps . 1 mark$'
Stop-Player
$kept = Get-Content -LiteralPath $settings
Confirm-That 'the switches are kept between runs' (($kept -contains 'screenshot_caption = on') -and ($kept -contains 'screenshot_markup = off') -and ($kept -contains 'screenshot_guides = off')) "the settings say: $($kept -match 'screenshot_')"
# a still with clear pixels: its saved frame keeps its alpha
Add-Type -AssemblyName System.Drawing
$clear = Join-Path $env:APPDATA 'clear.png'
$bmp = New-Object Drawing.Bitmap 16, 16
foreach ($x in 0..15) { foreach ($y in 0..15) { $bmp.SetPixel($x, $y, [Drawing.Color]::FromArgb($(if ($y -lt 8) { 0 } else { 255 }), 200, 100, 50)) } }
$bmp.Save($clear, [Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
Start-Player $clear
Assert-Title 'a still with clear pixels opens' '^clear\.png . 0 / 1 . 16 . 16 . png . still$'
$alpha = Join-Path $shots 'clear.f000000.png'
Send-Key 0x53
Confirm-That 'S keeps its alpha, and the caption switch, kept from the last run, puts the caption under it' ((Wait-File $alpha) -and (Get-PngColor $alpha) -eq 6 -and (Get-PngSize $alpha) -match '^16 x (\d+)$' -and [int]$Matches[1] -gt 16) "$alpha is $(if (Test-Path -LiteralPath $alpha) { "$(Get-PngSize $alpha), color type $(Get-PngColor $alpha)" })"
Stop-Player
foreach ($made in $marked, $plain, $captioned, $alpha, $shots, $clear) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}

# ---- the browser, in a tree of the test's own: a folder of two clips, three clips, a clip marked as not on the disk
# (as a cloud folder marks what it has not downloaded) and a file that is no video. the two folders above it hold
# nothing else, so the columns that fit show only what the test made
$wrap = Join-Path $env:APPDATA 'wrap'; $above = Join-Path $wrap 'in'; $tree = Join-Path $above 'browse'; $shots = Join-Path $tree 'shots'
$copies = [ordered]@{ (Join-Path $shots 'h264_320.mp4') = 'h264_320.mp4'; (Join-Path $shots 'wall_v012.mov') = 'hapq_64.mov'; (Join-Path $tree 'hap_64.mov') = 'hap_64.mov'; (Join-Path $tree 'prores_64.mov') = 'prores_64.mov'; (Join-Path $tree 'x_roll.mov') = 'hap_64.mov'; (Join-Path $tree 'z_away.mov') = 'hap_64.mov' }
New-Item -ItemType Directory -Force (Join-Path $env:APPDATA 'eyepiece') | Out-Null
New-Item -ItemType Directory $wrap, $above, $tree, $shots | Out-Null
foreach ($to in $copies.Keys) { Copy-Item -LiteralPath (Join-Path $media $copies[$to]) -Destination $to }
Set-Content -LiteralPath (Join-Path $tree 'notes.txt') -Value 'not a clip'
Set-Content -LiteralPath (Join-Path $tree 'y_broken.mov') -Value 'not a clip either, whatever its name says'
(Get-Item -LiteralPath (Join-Path $tree 'z_away.mov')).Attributes = 'Offline'
# the browser starts in the folder it was last in, which the settings name
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value "browser_folder = $tree", 'mute = on'
Start-Player
Assert-Title 'started with no file, the browser is up' '^eyepiece$'
# folders are listed off the drawing thread: each gets a moment to arrive before the keys move in it
Start-Sleep -Milliseconds 1000
# the cards on screen get their posters, kept in the cache: one file for each clip that is on the disk
function Get-Kept { @(Get-ChildItem -LiteralPath $previews -Filter *.previews -ErrorAction SilentlyContinue) }
foreach ($i in 1..100) { if ((Get-Kept).Count -ge 3) { break }; Start-Sleep -Milliseconds 100 }
Start-Sleep -Milliseconds 1000
Confirm-That 'each card whose file is on the disk gets a poster, and the one marked as not on the disk is not read' ((Get-Kept).Count -eq 3) "$((Get-Kept).Count) files of pictures in the cache, for 3 clips on the disk and 1 not"
$before = Get-Cpu; Start-Sleep -Seconds 2; $used = (Get-Cpu) - $before
Confirm-That 'the browser at rest, its folders listed and its posters made, uses no cpu' ($used -lt 0.05) "used $([math]::Round($used, 2)) s of cpu in 2 s"
# a double click opens the card its first click hit, though that click brings in the metadata column and moves the
# cards along. here the first card, beside as many columns as fit and no metadata column
$columns = [math]::Floor(((Get-Width) - 150 - 360) / 168); $main = 150 + 168 * $columns; $inner = (Get-Width) - $main - 28
$across = [math]::Floor(($inner + 12) / 172); $card = [math]::Floor(($inner - 12 * ($across - 1)) / $across)
$script:name = 'hap_64.mov'; DoubleClick-At ($main + 14 + $card / 2) (32 + 36 + 36 + 14 + $card * 9 / 32)
Assert-Frame 'a double click opens the card it was on' 0 3
# left from the grid's first card goes back to the columns, home selects the folder of shots there and the main area
# shows it; right enters it on its first clip, down goes to the next
Send-Key 0x4F; Start-Sleep -Milliseconds 500; Send-Key 0x25; Send-Key 0x24; Start-Sleep -Milliseconds 500; Send-Key 0x27; Send-Key 0x28
$script:name = 'wall_v012.mov'; Send-Key 0x0D; Assert-Frame 'the keys walk to a clip in a folder and enter opens it' 0 3
Send-Key 0x4F; Assert-Title 'O lays the browser over it' ' . browser$'
# backspace goes up a folder, where down selects the clip after the folder of shots
Start-Sleep -Milliseconds 500; Send-Key 0x08; Send-Key 0x28
$script:name = 'hap_64.mov'; Send-Key 0x0D; Assert-Frame 'backspace and enter on another clip open it' 0 3
Send-Key 0xDD; Assert-Frame 'in place of the first: ] finds no other tab' 0 3
# letters select by name. a posted key is typed as a keyboard's is, its character too. the player has no use for X,
# and the first letter after O must still reach the browser
Send-Key 0x4F; Start-Sleep -Milliseconds 500; Send-Key 0x58
$script:name = 'x_roll.mov'; Send-Key 0x0D; Assert-Frame 'a letter selects the first name that starts with it' 0 3
Send-Key 0x4F; Start-Sleep -Milliseconds 500; Send-Key 0x50; Send-Key 0x52
$script:name = 'prores_64.mov'; Send-Key 0x0D; Assert-Frame 'letters typed together select by the first letters of a name' 0 4
Send-Key 0x4F; Assert-Title 'the browser is over the clip again' ' . browser$'
# a double click on nothing opens nothing, however recently a clip was clicked: here its row, the fourth of the last
# column (the metadata column is up, so the columns that fit are fewer), then the empty grid at the bottom
$columns = [math]::Floor(((Get-Width) - 150 - 316 - 360) / 168)
Click-At (150 + 168 * $columns - 84) (32 + 36 + 8 + 26.25 * 3.5); Start-Sleep -Milliseconds 1000
DoubleClick-At (150 + 168 * $columns + 100) ((Get-Height) - 80); Start-Sleep -Milliseconds 1000
Assert-Title 'a double click on nothing opens nothing' ' . browser$'
Send-Key 0x1B; Assert-Frame 'escape goes back to the clip' 0 4
# the selected card's actions at the foot of the metadata column (fig. 28, less its "new tab", which the owner had
# no use for): open and reveal, 25 high and 12 from the bottom, from 15 in, 44.4 and 57.6 wide with 6 between.
# only open is pressed: reveal would put an explorer window in front of whoever is at the pc
Send-Key 0x4F; Assert-Title 'the browser is over the clip once more' ' . browser$'; Start-Sleep -Milliseconds 500
Click-At (-316 + 15 + 22) -24.5
Assert-Frame 'open, at the foot of the metadata column, opens the selected card' 0 4
Stop-Player
# the folders files were opened from, newest first: the tree's last, the folder of shots before
$kept = Get-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt')
Confirm-That 'the folders files were opened from are kept as the recent ones, the newest first' ($kept -contains "browser_recent = $tree | $shots") "the settings say: $($kept -match 'browser_recent')"
# reveal (fig. 28) is not pressed here: it would put an explorer window in front of whoever is at the pc

# ---- the address: a line across the top of the browser, right of the sidebar, with every folder from the drive
# down to the one shown. control + L makes it a field with the path selected: a folder's name typed over it goes into
# that folder, and the browser keeps where it is between runs, which is how this is read back
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value "browser_folder = $tree", 'mute = on'
Start-Player
Start-Sleep -Milliseconds 1000
# pressed again once the field has the keyboard, it types no letter into it
Send-Key 0x4C 1 4; Start-Sleep -Milliseconds 400; Send-Key 0x4C 1 4; Start-Sleep -Milliseconds 300
Send-Char 'shots'; Send-Key 0x0D; Start-Sleep -Milliseconds 800
Stop-Player
$kept = Get-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt')
Confirm-That 'a folder''s name typed into the address goes into that folder' ($kept -contains "browser_folder = $shots") "the settings say: $($kept -match 'browser_folder')"
# a press on a folder of the address goes to it: here the one above the folder shown. the names are 6.9 a letter in
# the address's type, from 108 in (after the three buttons), with 20.7 of " / " between them; the line's middle is
# 18 under the strip
Start-Player
Start-Sleep -Milliseconds 1000
$names = $shots -split '\\'
$x = 150 + 108; foreach ($part in $names[0..($names.Count - 3)]) { $x += $part.Length * 6.9 + 20.7 }
$crumb = $x + $names[-2].Length * 6.9 / 2
Click-At $crumb (32 + 18); Start-Sleep -Milliseconds 800
Stop-Player
$kept = Get-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt')
Confirm-That 'a press on a folder of the address goes to that folder' ($kept -contains "browser_folder = $tree") "the settings say: $($kept -match 'browser_folder')"
# a path that is not there: the browser says the folder cannot be read (the unit tests read the words), and the
# address takes the next path all the same, here a whole one
Start-Player
Start-Sleep -Milliseconds 1000
Send-Key 0x4C 1 4; Start-Sleep -Milliseconds 400; Send-Char 'nothere'; Send-Key 0x0D; Start-Sleep -Milliseconds 800
Send-Key 0x4C 1 4; Start-Sleep -Milliseconds 400; Send-Char $shots; Send-Key 0x0D; Start-Sleep -Milliseconds 800
Stop-Player
$kept = Get-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt')
Confirm-That 'after a path that is not there, the address goes to a whole path typed into it' ($kept -contains "browser_folder = $shots") "the settings say: $($kept -match 'browser_folder')"
# back and forward, the first two buttons at the left of the address: the browser goes along the folders it has
# shown. here from the tree into shots (typed) and to the tree again by its place in the address; two steps back and
# one forward end in shots, whichever way they are asked for
function Test-Retrace([string]$by, [scriptblock]$back, [scriptblock]$forward) {
    Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value "browser_folder = $tree", 'mute = on'
    Start-Player
    Start-Sleep -Milliseconds 1000
    Send-Key 0x4C 1 4; Start-Sleep -Milliseconds 400; Send-Char 'shots'; Send-Key 0x0D; Start-Sleep -Milliseconds 800
    Click-At $crumb (32 + 18); Start-Sleep -Milliseconds 800
    & $back; Start-Sleep -Milliseconds 500; & $back; Start-Sleep -Milliseconds 500; & $forward; Start-Sleep -Milliseconds 800
    Stop-Player
    $kept = Get-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt')
    Confirm-That "two steps back and one forward by $by end in the folder between" ($kept -contains "browser_folder = $shots") "the settings say: $($kept -match 'browser_folder')"
}
Test-Retrace 'the address''s buttons' { Click-At 171 50 } { Click-At 203 50 }
Test-Retrace 'alt with left and right' { Send-Key 0x25 1 2 } { Send-Key 0x27 1 2 }
# the side buttons say which of them in the high half of the message's first parameter, and carry the tests' mark
function Send-Side([int]$which) { [void][EyepieceE2e]::PostMessage($script:h, 0x20B, [IntPtr](($which -shl 16) -bor 0x8000), [IntPtr]0) }
Test-Retrace 'the mouse''s side buttons' { Send-Side 1 } { Send-Side 2 }
# read again, the third button, and f5 and control + R: a folder made since the browser listed the one it is in is
# not there until then. each is found by its first letter and entered, which the folder the browser ends in tells
$fresh = Join-Path $env:APPDATA 'fresh'; New-Item -ItemType Directory $fresh | Out-Null
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value "browser_folder = $fresh", 'mute = on'
Start-Player
Start-Sleep -Milliseconds 1000
$late = $fresh
foreach ($again in { Click-At 235 50 }, { Send-Key 0x74 }, { Send-Key 0x52 1 4 }) {
    $late = Join-Path $late 'late'; New-Item -ItemType Directory $late | Out-Null
    & $again; Start-Sleep -Milliseconds 700
    # the letter comes with its key, as from a keyboard: a character alone after control + R would be taken for R's own
    Send-Key 0x4C; Start-Sleep -Milliseconds 300; Send-Key 0x0D; Start-Sleep -Milliseconds 500
}
Stop-Player
foreach ($made in $late, (Split-Path $late), (Split-Path (Split-Path $late)), $fresh) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}
$kept = Get-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt')
Confirm-That 'the read-again button, f5 and control + R each show a folder made since the last listing' ($kept -contains "browser_folder = $late") "the settings say: $($kept -match 'browser_folder')"
# copy and paste in the field go by the clipboard, which is this pc's own: the check is left out when it holds more
# than plain text, which a test could not put back, and what it held is put back after
Add-Type -AssemblyName System.Windows.Forms
$held = [System.Windows.Forms.Clipboard]::GetDataObject()
$more = if ($held) { @($held.GetFormats() | Where-Object { $_ -notin 'System.String', 'UnicodeText', 'Text', 'OEMText', 'Locale' }) } else { @() }
if ($more.Count -eq 0) {
    $was = Get-Clipboard -Raw
    Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value "browser_folder = $shots", 'mute = on'
    Start-Player
    Start-Sleep -Milliseconds 1000
    Send-Key 0x4C 1 4; Start-Sleep -Milliseconds 400; Send-Key 0x43 1 4; Start-Sleep -Milliseconds 400
    $copied = Get-Clipboard -Raw
    Set-Clipboard -Value "`"$tree`""
    Send-Key 0x56 1 4; Start-Sleep -Milliseconds 300; Send-Key 0x0D; Start-Sleep -Milliseconds 800
    Stop-Player
    if ($was) { Set-Clipboard -Value $was } else { [System.Windows.Forms.Clipboard]::Clear() }
    $kept = Get-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt')
    Confirm-That 'control + C in the address copies the path selected in it' ($copied -eq $shots) 'the clipboard does not hold the path of the folder shown'
    Confirm-That 'control + V types a path into the address, in quotes as explorer copies one' ($kept -contains "browser_folder = $tree") "the settings say: $($kept -match 'browser_folder')"
} else { Write-Host 'skip  copy and paste in the address: the clipboard holds more than plain text, which this test would not put back' }

# ---- a played folder: shift + enter plays the folder the browser shows, its clips one after another in name order.
# here hap_64.mov (3 frames), prores_64.mov (4), x_roll.mov (3) and y_broken.mov, which is no clip at all; the one
# marked as not on the disk is left out
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value "browser_folder = $tree", 'mute = on'
Start-Player
Start-Sleep -Milliseconds 1000
function Get-ClipUp { if ((Get-Title) -match ' . clip (\d) of 4') { [int]$Matches[1] } else { 0 } }
function Go-FirstClip { foreach ($i in 1..5) { if ((Get-ClipUp) -eq 1) { break }; Send-Key 0x22; Start-Sleep -Milliseconds 500 } }
# the title is left alone during play: space stops it, and then it says where the folder is. it has played for
# longer than its three clips last, so it has gone round past the file that does not open
Send-Key 0x0D 1 1; Start-Sleep -Milliseconds 1500; Send-Key 0x20
Assert-Title 'shift + enter plays the folder, past the file that does not open, without the one not on the disk' ' / \d . .* . clip [123] of 4( . \d+ held at cuts)?$'
Go-FirstClip
Assert-Title 'the first clip is the first by name' '^hap_64\.mov . \d / 3 . .* . clip 1 of 4'
Send-Key 0x22; Assert-Title 'page down goes to the next clip' '^prores_64\.mov . .* . clip 2 of 4'
Send-Key 0x21; Assert-Title 'page up goes back' '^hap_64\.mov . .* . clip 1 of 4'
Send-Key 0x21; Assert-Title 'page up from the first clip goes round to the last, which does not open and says why' '^eyepiece . .*y_broken\.mov'
Send-Key 0x21; Assert-Title 'and page up again goes on to the clip before it' '^x_roll\.mov . .* . clip 3 of 4'
Go-FirstClip
# the folder's tab (fig. 14): "browse /" from 12 (8 letters 6.9 wide), then the count, "1 of 4", between two arrows
# 16.9 wide with their room. the arrows do what the page keys do
Click-At 134 16; Assert-Title 'the arrow after the tab''s count goes to the next clip' '^prores_64\.mov . .* . clip 2 of 4'
Click-At 76 16; Assert-Title 'the arrow before it goes back' '^hap_64\.mov . .* . clip 1 of 4'
# a click on the count lists the folder's clips under the tab: 8 in from the left and 8 under the strip, a heading,
# then rows 23 high. a click on the third row goes to the third clip; esc puts the list away
Click-At 105 16; Start-Sleep -Milliseconds 400; Click-At 100 133
Assert-Title 'a click on a row of the folder''s list goes to that clip' '^x_roll\.mov . \d / 3 . .* . clip 3 of 4'
Send-Key 0x1B
Go-FirstClip
# a folder opens on loop all, which the title leaves unsaid; R goes to off, one, and all again
Send-Key 0x52; Assert-Title 'R on a folder turns looping off' ' . loop off . clip 1 of 4'
Send-Key 0x20
Assert-Title 'with looping off the folder plays through once and stops on the last frame of its last clip that opens' '^x_roll\.mov . 2 / 3 . .* . loop off . clip 3 of 4'
Send-Key 0x52; Assert-Title 'R again loops the one clip' ' . loop one . clip 3 of 4'
Send-Key 0x20; Start-Sleep -Milliseconds 700; Send-Key 0x20
Assert-Title 'which play then stays in' '^x_roll\.mov . \d / 3 . .* . loop one . clip 3 of 4'
Send-Key 0x52
Wait-Title 'and R again is all: nothing about the loop in the title' { param($t) $t -match ' . clip 3 of 4' -and $t -notmatch ' loop ' } 'clip 3 of 4 and no loop mode'
Stop-Player
# a folder given to the player at launch plays as a played folder too
Start-Player $tree
Start-Sleep -Milliseconds 1500; Send-Key 0x20
Assert-Title 'a folder given at launch plays as a played folder' ' . clip [123] of 4( . \d+ held at cuts)?$'
Stop-Player
# what the section made, file by file, then its folders, each named before it goes. nothing recursive
foreach ($made in @($copies.Keys) + (Join-Path $tree 'notes.txt'), (Join-Path $tree 'y_broken.mov'), $shots, $tree, $above, $wrap) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}

# ---- a network folder (fig. 29), through this pc's own share: the browser lists it and reads none of its files, so no
# poster is made, until the note's button asks for its previews; then its two clips get theirs. the folders are as deep
# as the columns that fit, so these list only what the test made
$share = Join-Path $env:APPDATA 'share'; $deep = Join-Path $share 'in'; $net = Join-Path $deep 'net'
$netCopies = [ordered]@{ (Join-Path $net 'hap_64.mov') = 'hap_64.mov'; (Join-Path $net 'prores_64.mov') = 'prores_64.mov' }
New-Item -ItemType Directory $share, $deep, $net | Out-Null
foreach ($to in $netCopies.Keys) { Copy-Item -LiteralPath (Join-Path $media $netCopies[$to]) -Destination $to }
$unc = '\\localhost\' + $net.Substring(0, 1) + '$' + $net.Substring(2)
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value "browser_folder = $unc", 'mute = on'
$had = (Get-Kept).Count
Start-Player
Start-Sleep -Milliseconds 3000
Confirm-That 'a network folder is listed and none of its files is read: no poster is made' ((Get-Kept).Count -eq $had) "$((Get-Kept).Count - $had) files of pictures made"
# the button stands at the note's right, 220 wide ("make previews · reads 2 files", 29 letters 6.9 wide, and its room)
# and centered on the note's height: found by its white edge, the first bright line under the path line
$top = Get-FirstBright -140 118 -134 436
Confirm-That 'the network folder''s note offers its previews' ($top -gt 0) 'no button found at the right of the note'
if ($top -gt 0) { Click-At -137 ($top + 15) }
foreach ($i in 1..100) { if ((Get-Kept).Count -ge $had + 2) { break }; Start-Sleep -Milliseconds 100 }
Start-Sleep -Milliseconds 500
Confirm-That 'asked for, the network folder''s two clips get their posters' ((Get-Kept).Count -eq $had + 2) "$((Get-Kept).Count - $had) files of pictures made, for 2 clips"
Stop-Player
foreach ($made in @($netCopies.Keys) + $net, $deep, $share) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}

# ---- a render writes over the clip that is open: the new file takes its place, on the frame the old one was on.
# the copy is the test's own; the clip written over it is another of the test clips, 24 frames where it had 3
$render = Join-Path $env:APPDATA 'render.mov'
Copy-Item -LiteralPath (Join-Path $media 'hapq_64.mov') -Destination $render
Start-Player $render
Assert-Frame 'the clip that will be written over is open' 0 3
Send-Key 0x47 2; Assert-Frame 'on its last frame' 2 3
Copy-Item -LiteralPath (Join-Path $media 'h264_320.mp4') -Destination $render -Force
Assert-Title 'written over on disk, the new file is shown in its place, on the same frame' '^render\.mov . 2 / 24 . 320 . 180 . h\.264'
Stop-Player
Write-Host "removing $render"; Remove-Item -LiteralPath $render

# ---- sound: the keys and the bar's button set how loud the player is, and that is kept. the tests' settings are
# muted, and the clip stands still while they are not, so nothing is heard
$settings = Join-Path $env:APPDATA 'eyepiece\settings.txt'
Start-Player (Join-Path $media 'tone48_stereo.mov')
Assert-Frame 'a clip with sound is open' 0 24
# the sound button is the second from the right
Send-Key 0x28; Click-At -55 -32
Stop-Player
$kept = Get-Content -LiteralPath $settings
Confirm-That 'down turns the volume down a tenth, and a click on the sound button ends the mute' (($kept -contains 'volume = 0.7') -and ($kept -contains 'mute = off')) "the settings say: $($kept -match 'volume|mute')"
($kept -replace '^mute = off$', 'mute = on') | Set-Content -LiteralPath $settings
Start-Player (Join-Path $media 'tone48_stereo.mov')
Send-Key 0x20; Start-Sleep -Milliseconds 1500; Send-Key 0x20
Assert-Title 'muted, it plays with its sound as its clock, and stops when asked' '^tone48_stereo\.mov . \d+ / 24 '
Stop-Player
Confirm-That 'and the mute is kept' ((Get-Content -LiteralPath $settings) -contains 'mute = on') 'the settings do not say mute = on'

# ---- files that do not list all their frames up front: their packets are counted on opening. mkv and webm keep
# milliseconds, so their rate is derived; the fragmented mp4's table stops at its first fragment; and frames of two
# lengths. each has 12 frames with a key frame every 4: steps cross them, and play runs to the end and back again
foreach ($clip in @(@('h264_64.mkv', '30 fps derived'), @('h264_frag_64.mp4', '15360/512 fps'), @('vp9_64.webm', '30 fps derived'), @('h264_vfr_64.mp4', 'variable rate'))) {
    $file = $clip[0]; $n = [regex]::Escape($file); $r = [regex]::Escape($clip[1])
    Start-Player (Join-Path $media $file)
    Assert-Title "$file opens on frame 0 of all 12, at $($clip[1])" "^$n . 0 / 12 . 64 . 64 . .* . $r$"
    Send-Key 0x23; Assert-Title "${file}: end goes to the last frame" "^$n . 11 / 12 . .* . $r$"
    Send-Key 0x46 6; Assert-Title "${file}: six steps back, across a key frame" "^$n . 5 / 12 . .* . $r$"
    # with looping off, play forward stops on the last frame and backward on the first. a frame on the way that could
    # not be shown would be counted in the title
    Send-Key 0x24; Send-Key 0x52; Send-Key 0x20
    Assert-Title "${file}: plays to the last frame and stops there" "^$n . 11 / 12 . .* . $r( . \d+ late)? . loop off$"
    Send-Key 0x4A
    Assert-Title "${file}: plays backward to the first frame and stops there" "^$n . 0 / 12 . .* . $r( . \d+ late)? . loop off$"
    Send-Key 0x52
    Stop-Player
}

# ---- what cannot be opened says why, and the window stays
Start-Player (Join-Path $media 'no such clip.mov')
Assert-Title 'a missing file says so' '^eyepiece . .*no such clip\.mov'
Stop-Player

# ---- one player: a second launch hands its file to the window already open and exits, having made no window
# the tests' own player windows, by the class the player names after EYEPIECE_INSTANCE, whatever their titles
# (powershell would pass a plain $null as an empty title)
function Get-Players { $n = 0; $w = [IntPtr]::Zero; while (($w = [EyepieceE2e]::FindWindowEx([IntPtr]::Zero, $w, "eyepiece-$env:EYEPIECE_INSTANCE", [NullString]::Value)) -ne [IntPtr]::Zero) { $n++ }; $n }
# starts the program again: it must exit with 0 within a few seconds and leave one player window
function Start-Again([string]$what, [hashtable]$how) {
    $again = Start-Process $Exe -PassThru @how
    # a process that ends this soon keeps its exit code only for a handle asked for before it ends
    [void]$again.Handle
    $exited = $again.WaitForExit(5000)
    if (-not $exited) { $again.Kill() }
    Confirm-That $what ($exited -and $again.ExitCode -eq 0 -and (Get-Players) -eq 1) "exited: $exited, exit code $(if ($exited) { $again.ExitCode }), $(Get-Players) player window(s)"
}
# the first player runs in the tests folder, where the relative path given below names nothing
Push-Location $PSScriptRoot; Start-Player (Join-Path $media 'hapq_64.mov'); Pop-Location
Assert-Frame 'the first player opens its clip' 0 3
Start-Again 'a second launch with a file exits at once, and no second window opens' @{ ArgumentList = "`"$(Join-Path $media 'prores_64.mov')`"" }
$script:name = 'prores_64.mov'; Assert-Frame 'the first window now shows the second launch''s file' 0 4 'software'
Start-Again 'a launch with a path relative to its own folder exits at once too' @{ ArgumentList = 'hap_64.mov'; WorkingDirectory = $media }
$script:name = 'hap_64.mov'; Assert-Frame 'and the first window opens that path, resolved from the launch''s folder' 0 3
# each file handed over got a tab of its own, and never took the place of what was open: the first clip is two tabs back
Send-Key 0xDB 2; $script:name = 'hapq_64.mov'; Assert-Frame 'the first clip is still there, in its own tab' 0 3
# the first and the last tab hold clips of one size and length: they change places on the same frame, a flip comparison
Send-Key 0x23; Send-Key 0xDB; $script:name = 'hap_64.mov'; Assert-Frame 'two clips of one size and length change places on the same frame' 2 3
Send-Key 0xDD; $script:name = 'hapq_64.mov'; Assert-Frame 'and back' 2 3
# a file dropped on the window opens in a tab of its own too. the drop is posted as the shell sends one: a list of
# names in memory (a 20-byte header that says where the names start and that they are wide, the one name, two ends)
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class EyepieceDrop {
  [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
  [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr h);
  [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr h);
  public static IntPtr Files(string path) {
    byte[] name = Encoding.Unicode.GetBytes(path + "\0\0");
    IntPtr h = GlobalAlloc(0x42, (UIntPtr)(20 + name.Length));
    IntPtr at = GlobalLock(h);
    Marshal.WriteInt32(at, 0, 20); Marshal.WriteInt32(at, 16, 1);
    Marshal.Copy(name, 0, IntPtr.Add(at, 20), name.Length);
    GlobalUnlock(h);
    return h;
  }
}
'@
[void][EyepieceE2e]::PostMessage($script:h, 0x233, [EyepieceDrop]::Files((Join-Path $media 'h264_64.mp4')), [IntPtr]::Zero)
$script:name = 'h264_64.mp4'; Assert-Frame 'a file dropped on the window opens' 0 12
Send-Key 0xDD; $script:name = 'hapq_64.mov'; Assert-Frame 'in a tab of its own: what was open is still there, on its frame' 2 3
# a folder dropped on the window plays as a played folder, in a tab of its own: here the tests' own clips
[void][EyepieceE2e]::PostMessage($script:h, 0x233, [EyepieceDrop]::Files($media), [IntPtr]::Zero)
Start-Sleep -Milliseconds 1500; Send-Key 0x20
Assert-Title 'a folder dropped on the window plays as a played folder' ' . clip \d+ of \d+'
Send-Key 0xDB; $script:name = 'hapq_64.mov'; Assert-Frame 'in a tab of its own: the clip that was up is still there, on its frame' 2 3
[void][EyepieceE2e]::ShowWindow($script:h, 6); Start-Sleep -Milliseconds 400
$before = Get-Title
Start-Again 'a launch with no file exits at once' @{}
Start-Sleep -Milliseconds 400
Confirm-That 'and brings the minimized window back, its clip as it was' ((-not [EyepieceE2e]::IsIconic($script:h)) -and (Get-Title) -eq $before) "minimized: $([EyepieceE2e]::IsIconic($script:h)), title '$(Get-Title)', was '$before'"
Stop-Player

# the pictures the browser made, each named before it goes, then the folders made for them. nothing recursive
if (Test-Path -LiteralPath $previews) {
    foreach ($made in @(Get-ChildItem -LiteralPath $previews -File)) { Write-Host "removing $($made.FullName)"; Remove-Item -LiteralPath $made.FullName }
}
foreach ($made in $previews, (Split-Path $previews), (Split-Path (Split-Path $previews)), $env:EYEPIECE_CACHE) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}
# the tests' own settings: the file, then the two folders made for it, each named before it goes. nothing recursive
foreach ($made in (Join-Path $env:APPDATA 'eyepiece\settings.txt'), (Join-Path $env:APPDATA 'eyepiece'), $env:APPDATA) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}
if ($script:failed) { Write-Host "$($script:failed) check(s) failed"; exit 1 }
Write-Host 'all checks hold'
