# makes the tiny clips the tests read, and the referee picture for one of them. needs a full ffmpeg on the path
# (the hap, x264, x265 and prores encoders are not in the lgpl build the player ships with). the results are committed;
# rerun only to change them.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
function Out-Clip([string]$name) { Join-Path $here $name }

ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=60' -frames:v 3 -c:v hap -format hap -chunks 2 (Out-Clip 'hap_64.mov')
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=60000/1001' -frames:v 3 -c:v hap -format hap_q -chunks 4 (Out-Clip 'hapq_64.mov')
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30,format=rgba' -frames:v 2 -c:v hap -format hap_alpha (Out-Clip 'hapalpha_64.mov')
# ffmpeg's own decode of the first hap frame: the referee our blocks are checked against
ffmpeg -v error -y -i (Out-Clip 'hap_64.mov') -frames:v 1 -f rawvideo -pix_fmt rgb24 (Out-Clip 'hap_64.frame0.rgb')

# clips ffmpeg decodes. key frames close together, so steps cross them; b-frames, so frames are stored out of order
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30' -frames:v 12 -c:v libx264 -g 4 -bf 2 -pix_fmt yuv420p -an (Out-Clip 'h264_64.mp4')
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30' -frames:v 12 -c:v libx265 -x265-params 'keyint=4:min-keyint=4:bframes=2:log-level=error' -pix_fmt yuv420p10le -tag:v hvc1 -an (Out-Clip 'hevc10_64.mp4')
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=24' -frames:v 4 -c:v prores_ks -profile:v 3 -pix_fmt yuv422p10le -an (Out-Clip 'prores_64.mov')
# big enough for a gpu's decoder to take
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=320x180:r=30' -frames:v 24 -c:v libx264 -g 8 -bf 2 -pix_fmt yuv420p -an (Out-Clip 'h264_320.mp4')
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=320x180:r=30' -frames:v 24 -c:v libx265 -x265-params 'keyint=8:min-keyint=8:bframes=2:log-level=error' -pix_fmt yuv420p10le -tag:v hvc1 -an (Out-Clip 'hevc10_320.mp4')
# a stream gpus do not take (4:2:2, 10-bit), and a container that lists no frames up front
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30' -frames:v 6 -c:v libx264 -g 3 -pix_fmt yuv422p10le -an (Out-Clip 'h264_422_64.mp4')
ffmpeg -v error -y -i (Out-Clip 'h264_64.mp4') -c copy (Out-Clip 'h264_64.mkv')
# a clip cut from a longer one without re-encoding, starting between key frames: 24 samples stored, an edit list
# hides the first 6, 18 are shown
ffmpeg -v error -y -ss 0.2 -i (Out-Clip 'h264_320.mp4') -c copy (Out-Clip 'h264_cut_320.mp4')

# for the metadata panel. 12 frames at 30000/1001 (0.4004 s) with a drop-frame timecode track and two audio
# tracks: one 0.4 s long, within a frame of the picture, and one 0.3 s long, three frames short
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30000/1001' -f lavfi -i 'sine=f=440:r=8000:d=0.4' -f lavfi -i 'sine=f=880:r=8000:d=0.3' -map 0:v -map 1:a -map 2:a -frames:v 12 -c:v libx264 -g 4 -pix_fmt yuv420p -c:a pcm_s16le -ac:a:0 2 -timecode '01:00:00;00' (Out-Clip 'av_tc_64.mov')
# tagged bt.709 in the stream and the container, and untagged at 720 lines, where the assumption is bt.709
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30,setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709' -frames:v 2 -c:v libx264 -pix_fmt yuv420p -an (Out-Clip 'h264_709_64.mp4')
ffmpeg -v error -y -f lavfi -i 'color=c=gray:s=1280x720:r=30' -frames:v 2 -c:v libx264 -pix_fmt yuv420p -an (Out-Clip 'h264_720.mp4')
# tagged bt.709 and video range in the stream alone, with no tags in the container
ffmpeg -v error -y -i (Out-Clip 'h264_64.mp4') -c copy -bsf:v 'h264_metadata=colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1:video_full_range_flag=0' (Out-Clip 'h264_vui709_64.mp4')
# prores 4444 with alpha, tagged bt.709 in its frame header
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=24,format=yuva444p10le,setparams=color_primaries=bt709:color_trc=bt709:colorspace=bt709' -frames:v 1 -c:v prores_ks -profile:v 4 -pix_fmt yuva444p10le -an (Out-Clip 'prores4444_64.mov')
# clips with sound: 1 s of tone under a plain picture. aac in stereo with a different tone on each side, so the
# channels can be told apart; pcm in mono (8-bit: 16-bit would not fit the size these clips keep to)
ffmpeg -v error -y -f lavfi -i 'color=c=gray:s=64x64:r=24:d=1' -f lavfi -i 'aevalsrc=0.5*sin(2*PI*440*t)|0.5*sin(2*PI*660*t):s=48000:c=stereo:d=1' -c:v libx264 -pix_fmt yuv420p -c:a aac -b:a 128k (Out-Clip 'tone48_stereo.mov')
ffmpeg -v error -y -f lavfi -i 'color=c=gray:s=64x64:r=24:d=1' -f lavfi -i 'sine=frequency=440:sample_rate=44100:duration=1' -c:v libx264 -pix_fmt yuv420p -c:a pcm_u8 (Out-Clip 'tone44_mono.mov')
