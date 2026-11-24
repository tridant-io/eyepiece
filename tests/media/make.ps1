# makes the tiny clips the unit tests read, and the referee picture for one of them. needs a full ffmpeg on the path
# (the hap encoder is not in the lgpl build the player ships with). the results are committed; rerun only to change them.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=60' -frames:v 3 -c:v hap -format hap -chunks 2 "$here\hap_64.mov"
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=60000/1001' -frames:v 3 -c:v hap -format hap_q -chunks 4 "$here\hapq_64.mov"
ffmpeg -v error -y -f lavfi -i 'testsrc2=s=64x64:r=30,format=rgba' -frames:v 2 -c:v hap -format hap_alpha "$here\hapalpha_64.mov"
# ffmpeg's own decode of the first hap frame: the referee our blocks are checked against
ffmpeg -v error -y -i "$here\hap_64.mov" -frames:v 1 -f rawvideo -pix_fmt rgb24 "$here\hap_64.frame0.rgb"
