# builds the player and puts everything it needs to run in dist\eyepiece: the program, the four ffmpeg libraries it
# loads, and their license. the program is signed with azure artifact signing, the way tridant's other apps are.
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1            build, test, package, sign
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1 -Install   and copy it to %LOCALAPPDATA%\Programs\eyepiece
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1 -NoSign    for a machine without the signing setup
# signing needs AZURE_TENANT_ID, AZURE_CLIENT_ID and AZURE_CLIENT_SECRET in the user's environment, the windows sdk's
# signtool and microsoft's artifact signing client tools. none of their values is printed. nothing is deleted:
# files in dist and in the install folder are overwritten in place.
param([switch]$Install, [switch]$NoSign)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$ffmpeg = Join-Path $repo 'third_party\ffmpeg'
if (-not (Test-Path (Join-Path $ffmpeg 'bin\avformat-63.dll'))) { throw 'run scripts\setup.ps1 first' }

function Invoke-Checked {
    param([string]$What, [scriptblock]$Run)
    # 'Continue' for the call only: powershell 5.1 turns a native command's stderr into errors. the exit code is the verdict.
    $ErrorActionPreference = 'Continue'
    & $Run
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

Invoke-Checked 'the tests' { cargo test --quiet --manifest-path (Join-Path $repo 'Cargo.toml') }
Invoke-Checked 'the build' { cargo build --quiet --release --manifest-path (Join-Path $repo 'Cargo.toml') }

$dist = Join-Path $repo 'dist\eyepiece'
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item (Join-Path $repo 'target\release\eyepiece.exe') $dist -Force
# only what the program loads: avformat and avcodec, and the two they load in turn
foreach ($dll in 'avformat-63.dll', 'avcodec-63.dll', 'avutil-61.dll', 'swresample-7.dll') { Copy-Item (Join-Path $ffmpeg "bin\$dll") $dist -Force }
# ffmpeg is lgpl: its license travels with its libraries, which stay separate files a user may replace
Copy-Item (Join-Path $ffmpeg 'LICENSE.txt') (Join-Path $dist 'ffmpeg-LICENSE.txt') -Force

$exe = Join-Path $dist 'eyepiece.exe'
if (-not $NoSign) {
    foreach ($name in 'AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET') {
        # a shell older than the user variables does not have them, and the signing library reads this process's environment
        if (-not [Environment]::GetEnvironmentVariable($name, 'Process')) {
            $value = [Environment]::GetEnvironmentVariable($name, 'User')
            if (-not $value) { throw "$name is not set" }
            [Environment]::SetEnvironmentVariable($name, $value, 'Process')
        }
    }
    function Find-Newest([string]$Name, [string[]]$Roots) {
        # x64 only: the signing library has to match signtool's architecture
        $found = @($Roots | Where-Object { Test-Path -LiteralPath $_ } |
            ForEach-Object { Get-ChildItem -LiteralPath $_ -Recurse -File -Filter $Name -ErrorAction SilentlyContinue } |
            Where-Object { $_.FullName -notmatch '\\(x86|arm|arm64)\\' } | Sort-Object { $_.VersionInfo.FileVersionRaw } -Descending)
        if ($found.Count -eq 0) { throw "no $Name under $($Roots -join '; ')" }
        $found[0].FullName
    }
    $signtool = Find-Newest 'signtool.exe' @((Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'), (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\ArtifactSigningClientTools'))
    $dlib = Find-Newest 'Azure.CodeSigning.Dlib.dll' @((Join-Path ${env:ProgramFiles(x86)} 'Microsoft\ArtifactSigningClientTools'), (Join-Path $env:USERPROFILE '.nuget\packages\microsoft.artifactsigning.client'))
    $meta = Join-Path $PSScriptRoot 'artifact-signing.json'
    # the certificate behind a signature lives three days; the timestamp keeps the signature valid after that
    Invoke-Checked 'signing' { & $signtool sign /fd SHA256 /tr 'http://timestamp.acs.microsoft.com' /td SHA256 /dlib $dlib /dmdf $meta $exe | Out-Null }
    Invoke-Checked 'checking the signature' { & $signtool verify /pa $exe | Out-Null }
    $sig = Get-AuthenticodeSignature $exe
    Write-Host "[package] signed: $($sig.Status), $($sig.SignerCertificate.Subject)"
}
$size = [math]::Round((Get-ChildItem $dist -File | Measure-Object Length -Sum).Sum / 1MB, 1)
Write-Host "[package] $dist ($size MB): $((Get-ChildItem $dist -File | ForEach-Object Name) -join ', ')"

if ($Install) {
    $to = Join-Path $env:LOCALAPPDATA 'Programs\eyepiece'
    New-Item -ItemType Directory -Force $to | Out-Null
    Copy-Item (Join-Path $dist '*') $to -Force
    Write-Host "[package] installed to $to"
}
