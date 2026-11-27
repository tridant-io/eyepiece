# the readme's screenshots, retaken for every release (claude.md): each from its own player (scripts\shot.ps1: own
# settings, instance and mute, the window's own picture), into docs\images. the footage is the demo project that
# scripts\footage.ps1 makes, on X: (subst X: <that folder>), so every path the player shows is X:\..., nothing of this
# pc's. -Only takes a comma list of shot names to retake just those.
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\shots.ps1 [-Exe target\release\eyepiece.exe] [-Only hero,browser]
param([string]$Exe, [string]$Out, [string]$Only = '',
    [string]$Hero = 'X:\first_light\renders\milky_way_v003.mp4', [string]$Timebar = 'X:\first_light\renders\pillars_v012.mov',
    [string]$Zoom = 'X:\first_light\renders\sun_loops_v004.mov', [string]$ZoomAt = '640,360',
    [string]$Guides = 'X:\first_light\dome\01_dusk.mov', [string]$Meta = 'X:\first_light\renders\sun_loops_v004.mov',
    [string]$Loop = 'X:\first_light\renders\sun_loops_v004.mov', [string]$Mark = 'X:\first_light\dome\01_dusk.mov',
    [string]$Folder = 'X:\first_light\dome', [string]$Renders = 'X:\first_light\renders')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
if (-not $Exe) { $Exe = Join-Path $repo 'target\release\eyepiece.exe' }
if (-not $Out) { $Out = Join-Path $repo 'docs\images' }
if (-not (Test-Path -LiteralPath $Hero)) { throw "no ${Hero}: make the demo footage with scripts\footage.ps1 and subst X: to its folder" }
$shot = Join-Path $PSScriptRoot 'shot.ps1'
# where save image puts the saved frame; the newest png there becomes saved-frame.png
$saved = Join-Path $env:TEMP 'eyepiece-shots-saved'
New-Item -ItemType Directory -Force $Out, $saved | Out-Null
$want = @($Only -split ',' | Where-Object { $_ })
function Shot([string]$name, [string]$open, [string[]]$settings, [string]$steps, [hashtable]$more = @{}) {
    if ($want.Count -and $want -notcontains $name) { return $false }
    $sized = @{ W = 1280; H = 720 }
    foreach ($key in $more.Keys) { $sized[$key] = $more[$key] }
    $more = $sized
    if ($open) { $more.Open = $open }
    # no update check but in the settings and about pictures: a shot of an older build would show the update button
    & powershell -NoProfile -ExecutionPolicy Bypass -File $shot -Exe $Exe @more -Settings ((@('mute = on', 'update_check = off') + $settings) -join '|') -Steps $steps -Out (Join-Path $Out "$name.png") | Select-Object -Last 1
    $true
}
$x, $y = $ZoomAt.Split(',')
$stops = (1..8 | ForEach-Object { 'key:BB;wait:250' }) -join ';'
# the clip up, its controls pinned up, six frames along
$null = Shot 'hero' $Hero @('pin_controls = on') 'key:47;wait:200;key:47;wait:200;key:47;wait:200;key:47;wait:200;key:47;wait:200;key:47;wait:200;move:640,360;wait:800'
# the pointer resting on the timebar, two thirds along
$null = Shot 'timebar' $Timebar @('pin_controls = on') 'move:850,716;wait:400;move:852,716;wait:1800'
# eight stops in from fit about the pointer
$null = Shot 'zoom-navigator' $Zoom @() "move:$x,$y;$stops;wait:700;move:$([int]$x + 3),$([int]$y + 3);wait:900"
# the guides on, and the view menu open on the bar
$null = Shot 'guides' $Guides @('pin_controls = on') 'key:44;wait:200;key:43;wait:200;click:1129,693;wait:700'
# the metadata pinned
$null = Shot 'metadata' $Meta @() 'key:49;wait:1500'
# loop analysis, and the loop menu open
$null = Shot 'loop-analysis' $Loop @('pin_controls = on') 'key:41;wait:2500;click:955,693;wait:700'
# a played folder with its list open from its tab's clip name
$null = Shot 'played-folder' $Folder @('pin_controls = on') 'wait:2500;key:20;wait:300;move:230,16;click:230,16;wait:900'
# the browser on the renders, list and gallery, the first card picked, wide enough for every column of the path. the
# sidebar names this pc's pinned folders: it is cut off
if (Shot 'browser' '' @("browser_folder = $Renders", 'browser_view = both', 'browser_main_width = 380') 'wait:2500;key:28;wait:300;key:27;wait:1800' @{ W = 1440 }) {
    Add-Type -AssemblyName System.Drawing
    $file = Join-Path $Out 'browser.png'
    $full = [Drawing.Bitmap]::FromFile($file)
    # 150 of the 1440 at 100%, scaled with the picture on a scaled display
    $side = [int](150 * $full.Width / 1440)
    $cut = $full.Clone([Drawing.Rectangle]::new($side, 0, $full.Width - $side, $full.Height), $full.PixelFormat)
    $full.Dispose()
    $cut.Save($file, [Drawing.Imaging.ImageFormat]::Png)
    $cut.Dispose()
}
# markup on the dusk clip: the warm horizon boxed and named, the lens flares ringed, with the row of tools
# the tool row's places at 1280 wide: circle 413, text 477, yellow 554, white 654, save image 861
$marks = 'key:45;wait:500;drag:880,395,1268,505;wait:300;move:413,60;wait:400;click:413,60;click:554,60;drag:165,95,465,395;wait:300;move:477,60;wait:300;click:477,60;click:654,60;click:886,362;wait:300;char:sky too warm here;key:0D;wait:500'
$null = Shot 'markup' $Mark @() ($marks + ';move:640,600;wait:300')
# the same marks saved by save image, with the caption under the picture
if (Shot 'saved-frame' $Mark @('screenshot_caption = on', 'screenshot_markup = on', "screenshot_folder = $saved") ($marks + ';move:861,60;wait:400;click:861,60;wait:1500')) {
    $newest = Get-ChildItem $saved -Filter *.png | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $newest) { throw "save image saved nothing into $saved" }
    # saved at the clip's size: 1280 across is enough for the readme
    & ffmpeg -v error -y -i $newest.FullName -vf 'scale=1280:-1:flags=lanczos' (Join-Path $Out 'saved-frame.png')
    if ($LASTEXITCODE -ne 0) { throw 'ffmpeg could not scale the saved frame' }
}
# settings and about, on a settings folder that names no machine
$appdata = @{ AppData = 'X:\Users\you\AppData\Roaming' }
$null = Shot 'settings' $Hero @('play_on_open = on', 'update_check = on') 'key:20;wait:300;key:BC:4;wait:800' $appdata
$null = Shot 'about' $Hero @('update_check = on') 'wait:2000;click:1144,16;wait:900' $appdata
