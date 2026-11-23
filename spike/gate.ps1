# runs one gate program with the fetched ffmpeg and libclang on its path.
#   powershell -File spike\gate.ps1 hap_demux spike\media\hapq.mov
param(
    [Parameter(Mandatory = $true)][string]$Gate,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest
)
$tp = Join-Path $PSScriptRoot 'third_party'
$env:FFMPEG_DIR = Join-Path $tp 'ffmpeg'
$env:LIBCLANG_PATH = Join-Path $tp 'libclang'
$env:PATH = (Join-Path $tp 'ffmpeg\bin') + ';' + $env:PATH
cargo run --quiet --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') --release --bin $Gate -- @Rest
exit $LASTEXITCODE
