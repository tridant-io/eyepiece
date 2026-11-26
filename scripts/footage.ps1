# the readme's demo project from four free 4K sources, cut to six-second excerpts with no sound, in the codecs a
# review sees. ffmpeg reads only the bytes for the excerpt over https. writes into -Root (a new folder)
param([string]$Root)
$ErrorActionPreference = 'Stop'
$moon = 'https://svs.gsfc.nasa.gov/vis/a000000/a005500/a005587/phases_2026_plain_2160p30.mp4'
$sun = 'https://svs.gsfc.nasa.gov/vis/a010000/a012000/a012034/SDO_UHD_30mins_YouTube.mp4'
$pillars = 'https://cdn.esawebb.org/archives/videos/ultra_hd/weic2216d.mp4'
$paranal = 'https://cdn.eso.org/videos/ultra_hd/uhd_cm_paranal_3.mp4'
$hq = @('-c:v', 'prores_ks', '-profile:v', '3', '-pix_fmt', 'yuv422p10le', '-vendor', 'apl0')
$std = @('-c:v', 'prores_ks', '-profile:v', '2', '-pix_fmt', 'yuv422p10le', '-vendor', 'apl0')
$hevc = @('-c:v', 'libx265', '-preset', 'medium', '-crf', '16', '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1', '-x265-params', 'log-level=error')
$avc = @('-c:v', 'libx264', '-preset', 'slow', '-crf', '14', '-pix_fmt', 'yuv420p', '-g', '50')
$hapq = @('-c:v', 'hap', '-format', 'hap_q')
function Cut([string]$from, [double]$at, [string]$to, [string[]]$how, [string[]]$more = @()) {
    $out = Join-Path $Root $to
    New-Item -ItemType Directory -Force (Split-Path $out) | Out-Null
    $t = [Diagnostics.Stopwatch]::StartNew()
    & ffmpeg -hide_banner -loglevel error -y -ss $at -i $from -t 6 -an -map_metadata -1 @more @how $out
    if ($LASTEXITCODE -ne 0) { throw "ffmpeg failed on $to" }
    "{0,-34} {1,6} MB {2,5:n0} s" -f $to, [int]((Get-Item $out).Length / 1MB), $t.Elapsed.TotalSeconds
}
# renders: four subjects, four codecs
Cut $pillars 41 'first_light\renders\pillars_v012.mov' $hq
Cut $sun 494 'first_light\renders\sun_loops_v004.mov' $hq
Cut $moon 0 'first_light\renders\moon_phases_v007.mp4' $hevc
Cut $paranal 40 'first_light\renders\milky_way_v003.mp4' $avc
# the dome's playlist: rates and a size that do not all match
Cut $paranal 8 'first_light\dome\01_dusk.mov' $hapq
Cut $paranal 40 'first_light\dome\02_milky_way.mov' $std
Cut $sun 1743 'first_light\dome\03_sun.mov' $std
Cut $moon 0 'first_light\dome\04_moon.mov' $std @('-vf', 'scale=1920:1080:flags=lanczos')
