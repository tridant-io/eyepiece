# builds the program's icon from the rendered mark (docs\design\logo\eyepiece-render.png, made by
# docs\design\logo\3d\eyepiece.py):
#   assets\icon\eyepiece.ico   windows: 16, 20, 24, 32, 40, 48, 64, 96, 128 and 256 px
#   assets\icon\eyepiece.res   the same, as the build links it into the exe (build.rs)
#   assets\icon\eyepiece.png   512 px, for the mac's dock
#     powershell -NoProfile -ExecutionPolicy Bypass -File scripts\icon.ps1 [-Sheet out.png]
# 48 px and up are the render set in the app icon's body (the family's: a rounded body in the player's dark gray,
# docs\design\logo\family.md) and made small. 40 px and down are drawn here for their size, as the family's small
# layers are: the render made that small is a ring and a blur. -Sheet also writes every layer enlarged, to look at.
param([string]$Sheet)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path $PSScriptRoot -Parent
$to = Join-Path $root 'assets\icon'
New-Item -ItemType Directory -Force $to | Out-Null
$dark = [System.Drawing.ColorTranslator]::FromHtml('#161616')
function New-Canvas([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'; $g.InterpolationMode = 'HighQualityBicubic'; $g.PixelOffsetMode = 'HighQuality'
    $bmp, $g
}
function Brush([string]$color) { New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($color)) }

# ---- the render in the body, 1024 px: the body is the macos template's, a superellipse 824 across
$master, $g = New-Canvas 1024
$points = foreach ($i in 0..359) {
    $t = $i * [Math]::PI / 180; $c = [Math]::Cos($t); $s = [Math]::Sin($t)
    New-Object System.Drawing.PointF -ArgumentList ([single](512 + 412 * [Math]::Sign($c) * [Math]::Pow([Math]::Abs($c), 0.4))), ([single](512 + 412 * [Math]::Sign($s) * [Math]::Pow([Math]::Abs($s), 0.4)))
}
$g.FillPolygon((New-Object System.Drawing.SolidBrush $dark), [System.Drawing.PointF[]]$points)
$render = [System.Drawing.Image]::FromFile((Join-Path $root 'docs\design\logo\eyepiece-render.png'))
$g.DrawImage($render, 0, 0, 1024, 1024)
$render.Dispose(); $g.Dispose()

# made small by halves, then the last step: one step from 1024 to 48 would drop most of the picture
function Get-Small([System.Drawing.Bitmap]$from, [int]$size) {
    $now = $from
    while ($now.Width / 2 -ge $size) {
        $half, $g = New-Canvas ($now.Width / 2)
        $g.DrawImage($now, 0, 0, $half.Width, $half.Height); $g.Dispose()
        if ($now -ne $from) { $now.Dispose() }
        $now = $half
    }
    if ($now.Width -eq $size) { return $now }
    $out, $g = New-Canvas $size
    $g.DrawImage($now, 0, 0, $size, $size); $g.Dispose()
    if ($now -ne $from) { $now.Dispose() }
    $out
}

# ---- drawn for the size: the same eyepiece from the same side, in as few shapes as say it. the body fills the
# square but for a pixel; the bezel is a bright ring up and to the left; the barrel runs off down and to the right,
# its grip a shade off the body and its chrome end light; the glass is black with the light across its upper left
# and the one orange low on its right. `u` is one sixteenth of the size: everything is counted in those
function Get-Drawn([int]$size) {
    $bmp, $g = New-Canvas $size
    $u = $size / 16.0
    $edge = [single]1; $side = [single]($size - 2); $round = [single]($side * 0.22)
    $body = New-Object System.Drawing.Drawing2D.GraphicsPath
    foreach ($corner in @(@($edge, $edge, 180), @(($edge + $side - 2 * $round), $edge, 270), @(($edge + $side - 2 * $round), ($edge + $side - 2 * $round), 0), @($edge, ($edge + $side - 2 * $round), 90))) {
        $body.AddArc([single]$corner[0], [single]$corner[1], 2 * $round, 2 * $round, [single]$corner[2], 90)
    }
    $body.CloseFigure()
    $g.FillPath((New-Object System.Drawing.SolidBrush $dark), $body)
    $g.SetClip($body)
    $cx = 7.1 * $u; $cy = 7.1 * $u
    function Stroke([string]$color, [double]$width, [double]$x0, [double]$y0, [double]$x1, [double]$y1) {
        $pen = New-Object System.Drawing.Pen ([System.Drawing.ColorTranslator]::FromHtml($color)), ([single]$width)
        $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
        $g.DrawLine($pen, [single]$x0, [single]$y0, [single]$x1, [single]$y1); $pen.Dispose()
    }
    function Disc([string]$color, [double]$x, [double]$y, [double]$r) { $g.FillEllipse((Brush $color), [single]($x - $r), [single]($y - $r), [single](2 * $r), [single](2 * $r)) }
    function Arc([string]$color, [double]$width, [double]$r, [double]$from, [double]$sweep) {
        $pen = New-Object System.Drawing.Pen ([System.Drawing.ColorTranslator]::FromHtml($color)), ([single]$width)
        $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
        $g.DrawArc($pen, [single]($cx - $r), [single]($cy - $r), [single](2 * $r), [single](2 * $r), [single]$from, [single]$sweep); $pen.Dispose()
    }
    # the barrel, far end first: chrome, then the grip over it
    Stroke '#c9c9c9' (7.0 * $u) ($cx + 3.0 * $u) ($cy + 2.5 * $u) ($cx + 5.2 * $u) ($cy + 4.4 * $u)
    Stroke '#3a3a3a' (10.4 * $u) $cx $cy ($cx + 2.9 * $u) ($cy + 2.4 * $u)
    Stroke '#0c0c0c' (9.2 * $u) $cx $cy ($cx + 2.5 * $u) ($cy + 2.1 * $u)
    # the bezel: lit from the upper left, in shade low on the right
    Disc '#8f8f8f' $cx $cy (5.5 * $u)
    Disc '#f2f2f2' ($cx - 0.35 * $u) ($cy - 0.35 * $u) (5.15 * $u)
    # the glass, its light and the orange
    Disc '#050505' $cx $cy (3.45 * $u)
    Arc '#ffffff' ([Math]::Max(1.35, 1.05 * $u)) (2.1 * $u) 168 86
    Arc '#ff5a36' ([Math]::Max(1.5, 1.0 * $u)) (2.2 * $u) 4 68
    $g.Dispose()
    $bmp
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 96, 128, 256
$layers = @{}
foreach ($size in $sizes) { $layers[$size] = if ($size -le 40) { Get-Drawn $size } else { Get-Small $master $size } }

# ---- the .ico: each layer a png, which windows has taken at every size since vista
$pngs = @{}
foreach ($size in $sizes) { $stream = New-Object IO.MemoryStream; $layers[$size].Save($stream, [System.Drawing.Imaging.ImageFormat]::Png); $pngs[$size] = $stream.ToArray(); $stream.Dispose() }
$ico = New-Object IO.MemoryStream; $w = New-Object IO.BinaryWriter $ico
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$pngs.Count)
$at = 6 + 16 * $pngs.Count
foreach ($size in $sizes) {
    $w.Write([byte]($size % 256)); $w.Write([byte]($size % 256)); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$pngs[$size].Length); $w.Write([uint32]$at)
    $at += $pngs[$size].Length
}
foreach ($size in $sizes) { $w.Write($pngs[$size]) }
$w.Flush(); [IO.File]::WriteAllBytes((Join-Path $to 'eyepiece.ico'), $ico.ToArray()); $w.Dispose()

# ---- the .res the build links: the windows sdk's resource compiler makes it from the .ico
$rc = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\rc.exe' -ErrorAction SilentlyContinue | Sort-Object FullName | Select-Object -Last 1
if (-not $rc) { throw 'the windows sdk''s rc.exe is not on this pc: eyepiece.res is not made' }
$script = Join-Path $to 'eyepiece.rc'
Set-Content -LiteralPath $script -Value '1 ICON "eyepiece.ico"' -Encoding ascii
& $rc.FullName /nologo /fo (Join-Path $to 'eyepiece.res') $script
if ($LASTEXITCODE -ne 0) { throw "rc.exe ended with $LASTEXITCODE" }
Remove-Item -LiteralPath $script

# ---- the mac's dock
$dock = Get-Small $master 512
$dock.Save((Join-Path $to 'eyepiece.png'), [System.Drawing.Imaging.ImageFormat]::Png); $dock.Dispose()

if ($Sheet) {
    # every layer at its own size and enlarged to 192, pixels kept, on a dark and a light ground
    $count = $sizes.Count
    $sheetBmp = New-Object System.Drawing.Bitmap (40 + 212 * $count), 560
    $g = [System.Drawing.Graphics]::FromImage($sheetBmp)
    $g.FillRectangle((Brush '#202020'), 0, 0, $sheetBmp.Width, 280); $g.FillRectangle((Brush '#f2f2f2'), 0, 280, $sheetBmp.Width, 280)
    $g.InterpolationMode = 'NearestNeighbor'; $g.PixelOffsetMode = 'Half'
    $font = New-Object System.Drawing.Font 'Consolas', 9
    $i = 0
    foreach ($size in $sizes) {
        foreach ($row in 0, 280) {
            $x = 30 + 212 * $i
            $g.DrawImage($layers[$size], $x, $row + 20, 192, 192)
            if ($size -le 48) { $g.DrawImage($layers[$size], $x, $row + 222, $size, $size) }
            $g.DrawString("$size", $font, (Brush '#8a8a8a'), $x + 56, $row + 228)
        }
        $i++
    }
    $g.Dispose(); $sheetBmp.Save($Sheet); $sheetBmp.Dispose()
}
foreach ($layer in $layers.Values) { $layer.Dispose() }
$master.Dispose()
Get-ChildItem $to | ForEach-Object { '{0}  {1} KB' -f $_.Name, [int]($_.Length / 1KB) }
