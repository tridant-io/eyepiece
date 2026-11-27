# one readme screenshot: starts a player on scratch settings with -Open (a file or folder; none for the browser),
# sizes its picture area to -W x -H, follows -Steps, and saves the window's own picture (PrintWindow) to -Out.
# steps, separated by ';':  key:<vk>[:<mods 1 shift 2 alt 4 ctrl>]  char:<text>  wait:<ms>  move:x,y  click:x,y
# drag:x1,y1,x2,y2  (sizes and pointer positions are the client area's pixels at 100%: on a scaled display they are
# scaled to it, so the player lays out the same, and the picture is the display's own pixels, sharper)
param([string]$Exe, [string]$Open = '', [string]$Settings = '', [string]$AppData = '', [int]$W = 1280, [int]$H = 720, [string]$Steps = '', [string]$Out)
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing @"
using System; using System.Drawing; using System.Runtime.InteropServices;
public static class Shot {
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static void Save(IntPtr h, string file) {
    RECT c; GetClientRect(h, out c);
    using (var bmp = new Bitmap(c.R, c.B)) {
      using (var g = Graphics.FromImage(bmp)) { var dc = g.GetHdc(); PrintWindow(h, dc, 3); g.ReleaseHdc(dc); }
      bmp.Save(file, System.Drawing.Imaging.ImageFormat.Png);
    }
  }
}
"@
$env:EYEPIECE_POSTED_POINTER = '1'
# the window's real pixels: unaware, windows hands this script a scaled-down window and the picture is cut off
[void][Shot]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))
$env:APPDATA =if ($AppData) { $AppData } else { Join-Path $env:TEMP "eyepiece-shot-$PID" }
$env:EYEPIECE_CACHE = Join-Path $env:APPDATA 'os-cache'
$env:EYEPIECE_INSTANCE = "shot-$PID"
New-Item -ItemType Directory -Force (Join-Path $env:APPDATA 'eyepiece') | Out-Null
# marks drawn for a picture are notes, which are saved beside their clip (adr 0010): not here, where the clip is the
# owner's footage. -Settings 'notes_beside = on' turns it back on, for a clip in a folder of the caller's own
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value (@('mute = on', 'play_on_open = off', 'notes_beside = off') + @($Settings -split '\|' | Where-Object { $_ }))
$p = if ($Open) { Start-Process $Exe -ArgumentList "`"$Open`"" -PassThru } else { Start-Process $Exe -PassThru }
$hwnd = [IntPtr]::Zero
foreach ($i in 1..100) { Start-Sleep -Milliseconds 100; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $hwnd = $p.MainWindowHandle; break } }
# a file's first frame fits the window to the clip: sized after that, on the primary display
if ($Open) { foreach ($i in 1..50) { $p.Refresh(); if ($p.MainWindowTitle -ne 'eyepiece') { break }; Start-Sleep -Milliseconds 100 }; Start-Sleep -Milliseconds 500 }
$k = [Shot]::GetDpiForWindow($hwnd) / 96.0
foreach ($round in 1..3) {
  $c = New-Object Shot+RECT; $r = New-Object Shot+RECT; [void][Shot]::GetClientRect($hwnd, [ref]$c); [void][Shot]::GetWindowRect($hwnd, [ref]$r)
  [void][Shot]::SetWindowPos($hwnd, [IntPtr]::Zero, 40, 40, ($r.R - $r.L) + [int]($W * $k) - $c.R, ($r.B - $r.T) + [int]($H * $k) - $c.B, 0x14)
  Start-Sleep -Milliseconds 300
}
Start-Sleep -Milliseconds 1500
function At([string]$xy) { $v = $xy.Split(','); $x = [int]([double]$v[0] * $k); $y = [int]([double]$v[1] * $k); [IntPtr][int64](($y -shl 16) -bor ($x -band 0xffff)) }
foreach ($step in ($Steps -split ';' | Where-Object { $_ })) {
  $kind, $what = $step.Split(':', 2)
  switch ($kind) {
    'key' { $v = $what.Split(':'); $mods = if ($v.Count -gt 1) { [int]$v[1] } else { 0 }; [void][Shot]::PostMessage($hwnd, 0x100, [IntPtr][Convert]::ToInt32($v[0], 16), [IntPtr]$mods); Start-Sleep -Milliseconds 250 }
    'char' { foreach ($ch in $what.ToCharArray()) { [void][Shot]::PostMessage($hwnd, 0x102, [IntPtr][int]$ch, [IntPtr]0); Start-Sleep -Milliseconds 60 } }
    'wait' { Start-Sleep -Milliseconds ([int]$what) }
    'move' { [void][Shot]::PostMessage($hwnd, 0x200, [IntPtr]0x8000, (At $what)); Start-Sleep -Milliseconds 150 }
    'click' { $at = At $what; [void][Shot]::PostMessage($hwnd, 0x200, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 150; [void][Shot]::PostMessage($hwnd, 0x201, [IntPtr]0x8001, $at); Start-Sleep -Milliseconds 60; [void][Shot]::PostMessage($hwnd, 0x202, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 400 }
    'rclick' { $v = $what.Split(','); $at = At "$($v[0]),$($v[1])"; [void][Shot]::PostMessage($hwnd, 0x200, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 150; [void][Shot]::PostMessage($hwnd, 0x204, [IntPtr]0x8002, $at); Start-Sleep -Milliseconds 60; [void][Shot]::PostMessage($hwnd, 0x205, [IntPtr]0x8000, $at); Start-Sleep -Milliseconds 400 }
    'drag' {
      $v = $what.Split(','); $from = "$($v[0]),$($v[1])"; $to = "$($v[2]),$($v[3])"
      [void][Shot]::PostMessage($hwnd, 0x200, [IntPtr]0x8000, (At $from)); Start-Sleep -Milliseconds 100
      [void][Shot]::PostMessage($hwnd, 0x201, [IntPtr]0x8001, (At $from)); Start-Sleep -Milliseconds 100
      foreach ($i in 1..12) { $x = [int]$v[0] + ([int]$v[2] - [int]$v[0]) * $i / 12; $y = [int]$v[1] + ([int]$v[3] - [int]$v[1]) * $i / 12; [void][Shot]::PostMessage($hwnd, 0x200, [IntPtr]0x8001, (At "$([int]$x),$([int]$y)")); Start-Sleep -Milliseconds 30 }
      [void][Shot]::PostMessage($hwnd, 0x202, [IntPtr]0x8000, (At $to)); Start-Sleep -Milliseconds 300
    }
  }
}
Start-Sleep -Milliseconds 600
[Shot]::Save($hwnd, $Out)
"saved $Out"
[void]$p.CloseMainWindow(); [void]$p.WaitForExit(3000)
if (-not $p.HasExited) { $p.Kill() }
