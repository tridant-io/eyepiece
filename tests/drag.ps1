# dragging the window by its picture, with the real mouse: the one check the end-to-end test cannot make, because a
# move only starts while the real button is down.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\drag.ps1 [-Exe path\to\eyepiece.exe] [-Clip some.mov]
# it borrows the real pointer for about a second a case and puts it back. a hand on the mouse meanwhile is allowed
# for: the window is checked against where the pointer really went.
# the player's window is put on top of everything while it runs. exits 0 when every check holds.
param([string]$Exe = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\eyepiece.exe'),
    [string]$Clip = (Join-Path $PSScriptRoot 'media\h264_320.mp4'))
$ErrorActionPreference = 'Stop'
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class EyepieceDrag {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] public struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public MOUSEINPUT mi; }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int how);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int which);
  // the pointer is put at a place on the screen and the button pressed there as one event, so a hand on the mouse
  // cannot carry the press somewhere else in between
  public static void PressAt(int x, int y) {
    int left = GetSystemMetrics(76), top = GetSystemMetrics(77), w = GetSystemMetrics(78), h = GetSystemMetrics(79);
    var i = new INPUT[1]; i[0].type = 0;
    i[0].mi.dx = (int)Math.Round((x - left) * 65535.0 / (w - 1)); i[0].mi.dy = (int)Math.Round((y - top) * 65535.0 / (h - 1));
    i[0].mi.flags = 0x0001u | 0x0002u | 0x4000u | 0x8000u;
    SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
  public static void Release() {
    var i = new INPUT[1]; i[0].type = 0; i[0].mi.flags = 0x0004u;
    SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
  }
}
'@
[void][EyepieceDrag]::SetProcessDPIAware()
# this is the one test that uses the real mouse: the seam that makes the player deaf to it must be off
$env:EYEPIECE_POSTED_POINTER = $null
# the player's settings and its one-player-a-user group are the test's own
$env:APPDATA = Join-Path $env:TEMP "eyepiece-drag-$PID"
$env:EYEPIECE_INSTANCE = "drag-$PID"
$script:failed = 0

# starts a player, presses at a place in its window, moves the pointer 120 across and 72 down in twelve steps, and
# says how far the window went with it. the pointer is placed outright at each step, so its acceleration has no say
function Measure-Drag([string]$what, [int]$fromX, [int]$fromY, [switch]$Maximized) {
    $p = Start-Process $Exe -ArgumentList ('"' + $Clip + '"') -PassThru
    $h = [IntPtr]::Zero
    foreach ($i in 1..100) { Start-Sleep -Milliseconds 100; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break } }
    if ($h -eq [IntPtr]::Zero) { throw 'the player opened no window' }
    Start-Sleep -Milliseconds 800
    try {
        # on top of everything, without taking the keyboard
        [void][EyepieceDrag]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x0013)
        if ($Maximized) { [void][EyepieceDrag]::ShowWindow($h, 3); Start-Sleep -Milliseconds 500 }
        Start-Sleep -Milliseconds 200
        $before = New-Object EyepieceDrag+RECT; [void][EyepieceDrag]::GetWindowRect($h, [ref]$before)
        $was = New-Object EyepieceDrag+POINT; [void][EyepieceDrag]::GetCursorPos([ref]$was)
        $x = if ($fromX) { $before.L + $fromX } else { [int](($before.L + $before.R) / 2) }
        $y = if ($fromY) { $before.T + $fromY } else { [int](($before.T + $before.B) / 2) }
        [EyepieceDrag]::PressAt($x, $y)
        # where the press really landed, after the system's rounding
        $from = New-Object EyepieceDrag+POINT; [void][EyepieceDrag]::GetCursorPos([ref]$from)
        Start-Sleep -Milliseconds 80
        foreach ($i in 1..12) { [void][EyepieceDrag]::SetCursorPos($x + 10 * $i, $y + 6 * $i); Start-Sleep -Milliseconds 25 }
        Start-Sleep -Milliseconds 200
        # where the pointer really is by now (a hand on the mouse meanwhile moves it too), and the window with it
        $end = New-Object EyepieceDrag+POINT; [void][EyepieceDrag]::GetCursorPos([ref]$end)
        $held = New-Object EyepieceDrag+RECT; [void][EyepieceDrag]::GetWindowRect($h, [ref]$held)
        $pointer = ($end.X - $from.X), ($end.Y - $from.Y)
        [EyepieceDrag]::Release(); Start-Sleep -Milliseconds 150
        [void][EyepieceDrag]::SetCursorPos($was.X, $was.Y); Start-Sleep -Milliseconds 150
        $after = New-Object EyepieceDrag+RECT; [void][EyepieceDrag]::GetWindowRect($h, [ref]$after)
        $went = ($held.L - $before.L), ($held.T - $before.T)
        $stayed = ($after.L -eq $held.L) -and ($after.T -eq $held.T)
        if ($Maximized) {
            # a maximized window is put back to its own size as it starts to move, and then goes with the pointer
            $holds = (-not [EyepieceDrag]::IsZoomed($h)) -and $stayed -and (($held.R - $held.L) -lt ($before.R - $before.L))
            $detail = "maximized after: $([EyepieceDrag]::IsZoomed($h)); stayed put once let go: $stayed"
        } else {
            # to within two pixels: a hand on the mouse can move the pointer between the press and its being read here
            $holds = ([math]::Abs($went[0] - $pointer[0]) -le 2) -and ([math]::Abs($went[1] - $pointer[1]) -le 2) -and ($pointer[0] -ne 0) -and $stayed
            $detail = "the pointer went $($pointer[0]), $($pointer[1]) and the window $($went[0]), $($went[1]); stayed put once let go: $stayed"
        }
        if ($holds) { Write-Host "ok    $what" } else { Write-Host "FAIL  ${what}: $detail"; $script:failed++ }
    } finally {
        [void]$p.CloseMainWindow(); if (-not $p.WaitForExit(3000)) { $p.Kill() }
    }
}

Measure-Drag 'a drag on the picture moves the window with the point that was pressed' 0 0
Measure-Drag 'a drag on the strip moves it the same' 300 14
Measure-Drag 'a drag on the picture of a maximized window puts it back and moves it' 0 0 -Maximized

# the test's own settings: the file, then the two folders made for it, each named before it goes. nothing recursive
foreach ($made in (Join-Path $env:APPDATA 'eyepiece\settings.txt'), (Join-Path $env:APPDATA 'eyepiece'), $env:APPDATA) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}
if ($script:failed) { Write-Host "$($script:failed) check(s) failed"; exit 1 }
Write-Host 'all checks hold'
