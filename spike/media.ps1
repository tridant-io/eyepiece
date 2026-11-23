# generates the test clips the gates use into spike\media (ignored by git). needs an ffmpeg with the hap, prores_ks,
# hevc_nvenc and libx264 encoders on the path; the lgpl build under third_party only decodes, so this uses the system one.
# safe to re-run: it skips clips that exist, and it deletes nothing. the 8K clips are about 3 GB each.
param([switch]$Big)
$ErrorActionPreference = 'Stop'
$m = Join-Path $PSScriptRoot 'media'
New-Item -ItemType Directory -Force $m | Out-Null

function Make-Clip([string]$Name, [string[]]$FfArgs) {
    $out = Join-Path $m $Name
    if ((Test-Path $out) -and (Get-Item $out).Length -gt 0) { return }
    Write-Host "[media] $Name"
    & ffmpeg -hide_banner -loglevel error -y @FfArgs $out
    if ($LASTEXITCODE -ne 0) { Write-Host "[media] FAILED $Name" }
}

# testsrc2 alone compresses far better than real footage, so the "noise" clips add grain to get realistic packet sizes.
# every clip has a burned-in frame counter from testsrc2, so a decoded frame can be told from its neighbor.
$clean4k = 'testsrc2=size=3840x2160:rate=60'
$noise4k = 'testsrc2=size=3840x2160:rate=60,noise=alls=24:allf=t'
$noise8k = 'testsrc2=size=7680x4320:rate=60,noise=alls=24:allf=t'

Make-Clip 'hapq_4k60_8chunk.mov' @('-f', 'lavfi', '-i', $clean4k, '-t', '3', '-c:v', 'hap', '-format', 'hap_q', '-chunks', '8')
Make-Clip 'hapq_4k60_1chunk.mov' @('-f', 'lavfi', '-i', $clean4k, '-t', '3', '-c:v', 'hap', '-format', 'hap_q', '-chunks', '1')
Make-Clip 'hapq_4k60_noise.mov' @('-f', 'lavfi', '-i', $noise4k, '-t', '3', '-c:v', 'hap', '-format', 'hap_q', '-chunks', '8')
Make-Clip 'hap_4k60_noise.mov' @('-f', 'lavfi', '-i', $noise4k, '-t', '3', '-c:v', 'hap', '-format', 'hap', '-chunks', '8')
Make-Clip 'hapalpha_4k60_noise.mov' @('-f', 'lavfi', '-i', $noise4k, '-t', '3', '-pix_fmt', 'rgba', '-c:v', 'hap', '-format', 'hap_alpha', '-chunks', '8')

# hevc main10, hardware-encoded. one clip at 59.94 and one at 60.000, to see both cadences against the display.
$hevc = @('-pix_fmt', 'p010le', '-c:v', 'hevc_nvenc', '-profile:v', 'main10', '-preset', 'p5', '-b:v', '40M', '-g', '60', '-bf', '2',
    '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', '-color_range', 'tv', '-tag:v', 'hvc1')
Make-Clip 'hevc10_4k_5994.mp4' (@('-f', 'lavfi', '-i', 'testsrc2=size=3840x2160:rate=60000/1001', '-t', '10') + $hevc)
Make-Clip 'hevc10_4k_60.mp4' (@('-f', 'lavfi', '-i', 'testsrc2=size=3840x2160:rate=60', '-t', '10') + $hevc)

# h.264 with a long group, the awkward case for stepping back
Make-Clip 'h264_4k60_gop250.mp4' @('-f', 'lavfi', '-i', $clean4k, '-t', '10', '-pix_fmt', 'yuv420p', '-c:v', 'libx264', '-preset', 'veryfast', '-g', '250', '-bf', '2',
    '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709')

# prores, software decode on windows
Make-Clip 'prores_422hq_4k60.mov' @('-f', 'lavfi', '-i', $noise4k, '-t', '3', '-pix_fmt', 'yuv422p10le', '-c:v', 'prores_ks', '-profile:v', '3')
Make-Clip 'prores_4444xq_4k60.mov' @('-f', 'lavfi', '-i', $noise4k, '-t', '2', '-pix_fmt', 'yuv444p12le', '-c:v', 'prores_ks', '-profile:v', '5')

if ($Big) {
    Make-Clip 'hapq_8k60_noise.mov' @('-f', 'lavfi', '-i', $noise8k, '-t', '1.5', '-c:v', 'hap', '-format', 'hap_q', '-chunks', '16')
    Make-Clip 'hapq_8k60_noise_1chunk.mov' @('-f', 'lavfi', '-i', $noise8k, '-t', '1.5', '-c:v', 'hap', '-format', 'hap_q', '-chunks', '1')
}

Get-ChildItem $m -File | Sort-Object Name | ForEach-Object { '{0,9:N0} MB  {1}' -f ($_.Length / 1MB), $_.Name }
