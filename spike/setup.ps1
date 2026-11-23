# fetches the build pieces the gate programs need into spike\third_party (ignored by git).
# safe to re-run: it skips what is already there, and it deletes nothing.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$root = Join-Path $PSScriptRoot 'third_party'
New-Item -ItemType Directory -Force $root | Out-Null

# ffmpeg: lgpl, shared libraries, release branch 9.0 (adr 0005)
$ff = Join-Path $root 'ffmpeg'
if (-not (Test-Path (Join-Path $ff 'include\libavformat\avformat.h'))) {
    $zip = Join-Path $root 'ffmpeg-n9.0-win64-lgpl-shared.zip'
    Write-Host '[setup] downloading ffmpeg n9.0 lgpl shared'
    Invoke-WebRequest 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n9.0-latest-win64-lgpl-shared-9.0.zip' -OutFile $zip
    $tmp = Join-Path $root 'ffmpeg-unzip'
    Expand-Archive $zip -DestinationPath $tmp -Force
    $inner = Get-ChildItem $tmp -Directory | Select-Object -First 1
    Move-Item $inner.FullName $ff
}

# libclang, which bindgen needs to read ffmpeg's headers. the pypi "libclang" wheel is a zip that holds the dll.
$lc = Join-Path $root 'libclang'
if (-not (Test-Path (Join-Path $lc 'libclang.dll'))) {
    Write-Host '[setup] downloading libclang'
    $meta = Invoke-RestMethod 'https://pypi.org/pypi/libclang/json'
    $url = ($meta.urls | Where-Object { $_.filename -like '*win_amd64.whl' } | Select-Object -First 1).url
    $whl = Join-Path $root 'libclang-win_amd64.zip'
    Invoke-WebRequest $url -OutFile $whl
    $tmp = Join-Path $root 'libclang-unzip'
    Expand-Archive $whl -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Force $lc | Out-Null
    $dll = Get-ChildItem $tmp -Recurse -Filter 'libclang.dll' | Select-Object -First 1
    if (-not $dll) { throw 'libclang.dll not found inside the wheel' }
    Copy-Item $dll.FullName $lc
}

Write-Host ('[setup] ' + (& (Join-Path $ff 'bin\ffmpeg.exe') -hide_banner -version | Select-Object -First 1))
Write-Host ('[setup] libclang ' + [math]::Round((Get-Item (Join-Path $lc 'libclang.dll')).Length / 1MB) + ' MB')
Write-Host '[setup] import libs: ' -NoNewline
Write-Host ((Get-ChildItem (Join-Path $ff 'lib') -Filter '*.lib' | ForEach-Object Name) -join ' ')
