# what the timebar's picture takes in the real program, on clips of your own: from the pointer coming onto the
# timebar to the first picture over it, and to the picture that takes its place (the frame itself, or in a clip with
# long groups the key frame nearest it) when another comes.
#   powershell -NoProfile -ExecutionPolicy Bypass -Command "& tests\hover.ps1 -Clips a.mov,b.mp4 [-Runs 5] [-Settle 8]"
# measured from outside: the pointer is posted onto the timebar and the window's own picture (PrintWindow) is read
# over and over, until the picture's place above the bar is lit, then until it changes or 2 s go by. a reading is good
# to the time one look takes, which is printed with it. the clip stands still, its strip given -Settle seconds to be
# made first; each hover is at another place, with the pointer taken off the timebar between. prints numbers; nothing
# is judged. a window opens.
param([Parameter(Mandatory = $true)][string[]]$Clips, [int]$Runs = 5, [int]$Settle = 8, [string]$Exe)
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\eyepiece.exe' }
Add-Type -ReferencedAssemblies System.Drawing @'
using System; using System.Drawing; using System.Drawing.Imaging; using System.Runtime.InteropServices;
public static class EyepieceHover {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int ht, uint flags);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out Rect r);
  [StructLayout(LayoutKind.Sequential)] struct Rect { public int L, T, R, B; }
  // the red, green and blue of a part of the window's own picture
  public static byte[] Look(IntPtr h, int x0, int y0, int w, int ht) {
    Rect c; GetClientRect(h, out c);
    using (var bmp = new Bitmap(c.R, c.B, PixelFormat.Format24bppRgb)) {
      using (var g = Graphics.FromImage(bmp)) { var dc = g.GetHdc(); PrintWindow(h, dc, 3); g.ReleaseHdc(dc); }
      var d = bmp.LockBits(new Rectangle(x0, y0, w, ht), ImageLockMode.ReadOnly, PixelFormat.Format24bppRgb);
      var row = new byte[w * 3]; var all = new byte[w * 3 * ht];
      for (int y = 0; y < ht; y++) { Marshal.Copy(d.Scan0 + y * d.Stride, row, 0, w * 3); Array.Copy(row, 0, all, y * w * 3, w * 3); }
      bmp.UnlockBits(d);
      return all;
    }
  }
  // pixels that are not near black
  public static int Lit(byte[] p) { int n = 0; for (int i = 0; i < p.Length; i += 3) if (Math.Max(p[i], Math.Max(p[i + 1], p[i + 2])) > 60) n++; return n; }
  // the mean change of a channel between two looks
  public static double Change(byte[] a, byte[] b) { long s = 0; for (int i = 0; i < a.Length; i++) s += Math.Abs(a[i] - b[i]); return (double)s / a.Length; }
}
'@
# players of this script's own: posted pointer messages, its own settings (muted, and a browser folder of its own),
# and never handed to one the owner has open
$env:EYEPIECE_POSTED_POINTER = '1'
$env:EYEPIECE_INSTANCE = "hover-$PID"
$env:APPDATA = Join-Path $env:TEMP "eyepiece-hover-$PID"
New-Item -ItemType Directory -Force (Join-Path $env:APPDATA 'eyepiece') | Out-Null
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value 'mute = on', "browser_folder = $env:APPDATA"
foreach ($clip in $Clips) {
    $p = Start-Process $Exe -ArgumentList "`"$clip`"" -PassThru
    $h = [IntPtr]::Zero
    foreach ($i in 1..100) { Start-Sleep -Milliseconds 100; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break } }
    # tall, so the picture's place (82 to 172 up from the bottom, 160 wide over the pointer) is over black, not over a
    # 16:9 clip, and small, so a look is quick
    $scale = [EyepieceHover]::GetDpiForWindow($h) / 96.0
    [void][EyepieceHover]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, [int](640 * $scale), [int](760 * $scale), 0x0006)
    Start-Sleep -Seconds $Settle
    $post = { param($x, $y) [void][EyepieceHover]::PostMessage($h, 0x200, [IntPtr]0x8000, [IntPtr][int64](([int]($y * $scale) -shl 16) -bor [int]($x * $scale))) }
    $rows = foreach ($run in 1..$Runs) {
        & $post 320 380; Start-Sleep -Milliseconds 1200
        $x = 640 * ((2 * $run - 1) / (2 * $Runs + 1) + 0.03)
        $left = [math]::Min([math]::Max($x - 82, 4), 640 - 168)
        $look = { [EyepieceHover]::Look($h, [int](($left + 12) * $scale), [int]((760 - 165) * $scale), [int](140 * $scale), [int](75 * $scale)) }
        $sw = [Diagnostics.Stopwatch]::StartNew()
        & $post $x 755
        $first = $null; $then = $null; $looks = 0
        while ($sw.ElapsedMilliseconds -lt 2000) {
            $was = $sw.Elapsed.TotalMilliseconds; $now = & $look; $looks++
            if (-not $first) { if ([EyepieceHover]::Lit($now) -gt 1000) { $first = $sw.Elapsed.TotalMilliseconds; $at = $now; $spent = $first - $was } }
            elseif ([EyepieceHover]::Change($at, $now) -gt 2) { $then = $sw.Elapsed.TotalMilliseconds; break }
        }
        [pscustomobject]@{ first = $first; then = $then; look = ($sw.Elapsed.TotalMilliseconds / $looks) }
        # the pointer leaves the timebar straight up, so it does not cross to another frame on the way
        & $post $x 380
    }
    $f = @($rows | Where-Object first | ForEach-Object { [math]::Round($_.first) })
    $t = @($rows | Where-Object then | ForEach-Object { [math]::Round($_.then) })
    "{0}: first picture {1} ms; another took its place in {2} of {3} ({4} ms); one look took {5} ms" -f (Split-Path $clip -Leaf), ($f -join ' '), $t.Count, $Runs, ($t -join ' '), [math]::Round(($rows | Measure-Object look -Average).Average)
    [void]$p.CloseMainWindow(); if (-not $p.WaitForExit(3000)) { $p.Kill() }
}
# the script's own settings: the file, then the two folders made for it, each named before it goes. nothing recursive
foreach ($made in (Join-Path $env:APPDATA 'eyepiece\settings.txt'), (Join-Path $env:APPDATA 'eyepiece'), $env:APPDATA) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}
