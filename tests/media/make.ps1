# makes the tiny clips the unit tests read, and the referee picture for one of them. needs a full ffmpeg on the path
# (the hap encoder is not in the lgpl build the player ships with). the results are committed; rerun only to change them.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=60' -frames:v 3 -c:v hap -format hap -chunks 2 "$here\hap_64.mov"
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=60000/1001' -frames:v 3 -c:v hap -format hap_q -chunks 4 "$here\hapq_64.mov"
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30,format=rgba' -frames:v 2 -c:v hap -format hap_alpha "$here\hapalpha_64.mov"
# ffmpeg's own decode of the first hap frame: the referee our blocks are checked against
ffmpeg -v error -y -i "$here\hap_64.mov" -frames:v 1 -f rawvideo -pix_fmt rgb24 "$here\hap_64.frame0.rgb"

# clips ffmpeg decodes. key frames close together, so steps cross them; b-frames, so frames are stored out of order
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30' -frames:v 12 -c:v libx264 -g 4 -bf 2 -pix_fmt yuv420p -an "$hereh264_64.mp4"
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30' -frames:v 12 -c:v libx265 -x265-params 'keyint=4:min-keyint=4:bframes=2:log-level=error' -pix_fmt yuv420p10le -tag:v hvc1 -an "$herehevc10_64.mp4"
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=24' -frames:v 4 -c:v prores_ks -profile:v 3 -pix_fmt yuv422p10le -an "$hereprores_64.mov"
# big enough for a gpu's decoder to take
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=320x180:r=30' -frames:v 24 -c:v libx264 -g 8 -bf 2 -pix_fmt yuv420p -an "$hereh264_320.mp4"
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=320x180:r=30' -frames:v 24 -c:v libx265 -x265-params 'keyint=8:min-keyint=8:bframes=2:log-level=error' -pix_fmt yuv420p10le -tag:v hvc1 -an "$herehevc10_320.mp4"
# a stream gpus do not take (4:2:2, 10-bit), and a container that lists no frames up front
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30' -frames:v 6 -c:v libx264 -g 3 -pix_fmt yuv422p10le -an "$hereh264_422_64.mp4"
ffmpeg -v error -y -i "$hereh264_64.mp4" -c copy "$hereh264_64.mkv"
