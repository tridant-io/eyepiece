# how long from starting the program to its first picture, on clips of your own.
#   powershell -NoProfile -ExecutionPolicy Bypass -Command "& tests\launch.ps1 -Clips a.mov,b.mp4 [-Runs 6]"
# measured from outside: from the moment the system created the process to the moment the title names frame 0
# without " . reading" (the title is set just after that frame is presented). the title is polled every millisecond
# or so, so each reading is good to a few milliseconds and is, if anything, late. the first run of a clip is listed
# apart: it pays for cold caches. prints numbers; nothing is judged. windows open and close.
param([Parameter(Mandatory = $true)][string[]]$Clips, [int]$Runs = 6, [string]$Exe)
$ErrorActionPreference = 'Stop'
if (-not $Exe) { $Exe = Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\eyepiece.exe' }
# players of this script's own: never handed to one the owner has open, and with settings of their own
$env:EYEPIECE_INSTANCE = "launch-$PID"
$env:APPDATA = Join-Path $env:TEMP "eyepiece-launch-$PID"
# clips open paused, as they always have here: the measurement starts play itself
New-Item -ItemType Directory -Force (Join-Path $env:APPDATA 'eyepiece') | Out-Null
Set-Content -LiteralPath (Join-Path $env:APPDATA 'eyepiece\settings.txt') -Value 'play_on_open = off'
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class EyepieceLaunch {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  delegate bool EnumProc(IntPtr h, IntPtr l);
  // the first top-level window of a process whose title is not empty
  public static string Title(uint pid) {
    string found = null;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p != pid) return true;
      var s = new StringBuilder(512); GetWindowText(h, s, 512); if (s.Length == 0) return true; found = s.ToString(); return false; }, IntPtr.Zero);
    return found;
  }
}
'@
foreach ($clip in $Clips) {
    $name = [regex]::Escape((Split-Path $clip -Leaf))
    $times = foreach ($run in 0..$Runs) {
        $p = Start-Process $Exe -ArgumentList "`"$clip`"" -PassThru
        $ms = -1
        $limit = [Diagnostics.Stopwatch]::StartNew()
        while ($limit.ElapsedMilliseconds -lt 15000) {
            $t = [EyepieceLaunch]::Title([uint32]$p.Id)
            if ($t -and $t -match "^$name . 0 / " -and $t -notmatch 'reading') { $ms = [math]::Round(([DateTime]::Now - $p.StartTime).TotalMilliseconds); break }
            [Threading.Thread]::Sleep(1)
        }
        Start-Sleep -Milliseconds 300
        [void]$p.CloseMainWindow(); if (-not $p.WaitForExit(3000)) { $p.Kill() }
        Start-Sleep -Milliseconds 300
        $ms
    }
    $rest = @($times | Select-Object -Skip 1 | Sort-Object)
    "{0,-28} first launch {1,4} ms, then {2} (median {3} ms)" -f (Split-Path $clip -Leaf), $times[0], ($rest -join ' '), $rest[[math]::Floor($rest.Count / 2)]
}
# the script's own settings: the file, then the two folders made for it, each named before it goes. nothing recursive
foreach ($made in (Join-Path $env:APPDATA 'eyepiece\settings.txt'), (Join-Path $env:APPDATA 'eyepiece'), $env:APPDATA) {
    if (Test-Path -LiteralPath $made) { Write-Host "removing $made"; Remove-Item -LiteralPath $made }
}
