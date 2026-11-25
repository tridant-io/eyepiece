# builds the player and puts everything it needs to run in dist\eyepiece: the program, the four ffmpeg libraries it
# loads, their license, and the third-party notices. the program and the four libraries are signed with azure artifact
# signing, the way tridant's other apps are. then inno setup makes the windows installer of that folder,
# dist\eyepiece-<version>-windows-setup.exe (scripts\installer.iss), signed the same way with its uninstaller.
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1            build, test, package, sign
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1 -Install   and copy it to %LOCALAPPDATA%\Programs\eyepiece
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1 -NoSign    for a machine without the signing setup
# signing needs AZURE_TENANT_ID, AZURE_CLIENT_ID and AZURE_CLIENT_SECRET in the user's environment, the windows sdk's
# signtool and microsoft's artifact signing client tools. none of their values is printed. the installer needs inno
# setup 6. nothing is deleted: files in dist and in the install folder are overwritten in place.
param([switch]$Install, [switch]$NoSign)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$ffmpeg = Join-Path $repo 'third_party\ffmpeg'
if (-not (Test-Path (Join-Path $ffmpeg 'bin\avformat-63.dll'))) { throw 'run scripts\setup.ps1 first' }
$iscc = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'
if (-not (Test-Path -LiteralPath $iscc)) { throw "no inno setup 6 at $iscc" }
if ([IO.File]::ReadAllText((Join-Path $repo 'Cargo.toml')) -notmatch '(?m)^version = "(\d+\.\d+\.\d+)"') { throw 'Cargo.toml names no version x.y.z' }
$version = $Matches[1]

function Invoke-Checked {
    param([string]$What, [scriptblock]$Run)
    # 'Continue' for the call only: powershell 5.1 turns a native command's stderr into errors. the exit code is the verdict.
    $ErrorActionPreference = 'Continue'
    & $Run
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

Invoke-Checked 'the tests' { cargo test --quiet --manifest-path (Join-Path $repo 'Cargo.toml') }
Invoke-Checked 'the build' { cargo build --quiet --release --manifest-path (Join-Path $repo 'Cargo.toml') }
# the real program, driven by real keys: a window opens for a few seconds
Invoke-Checked 'the end-to-end test' { powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repo 'tests\e2e.ps1') }

$dist = Join-Path $repo 'dist\eyepiece'
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item (Join-Path $repo 'target\release\eyepiece.exe') $dist -Force
# only what the program loads: avformat, avcodec and avutil, and swresample, which avcodec loads and the audio code calls
foreach ($dll in 'avformat-63.dll', 'avcodec-63.dll', 'avutil-61.dll', 'swresample-7.dll') { Copy-Item (Join-Path $ffmpeg "bin\$dll") $dist -Force }
# ffmpeg is lgpl: its license travels with its libraries, which stay separate files a user may replace
Copy-Item (Join-Path $ffmpeg 'LICENSE.txt') (Join-Path $dist 'ffmpeg-LICENSE.txt') -Force
# the two typefaces drawn in the controls are built into the program; the open font license asks that it travel with them
Copy-Item (Join-Path $repo 'assets/fonts/IBM-Plex-LICENSE.txt') (Join-Path $dist 'fonts-IBM-Plex-LICENSE.txt') -Force
Copy-Item (Join-Path $repo 'assets/fonts/Archivo-OFL.txt') (Join-Path $dist 'fonts-Archivo-OFL.txt') -Force
# every crate built into the program, ffmpeg and the typefaces, with their licenses; -Install copies it with the rest
& (Join-Path $PSScriptRoot 'notices.ps1') -Out (Join-Path $dist 'THIRD-PARTY-NOTICES.txt') -Ffmpeg $ffmpeg

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
    # the four ffmpeg libraries with the program, so an update can prove each is tridant's (adr 0007). the player
    # never checks them as it loads them, so a user's own ffmpeg still runs, as the lgpl wants
    $signed = @($exe) + @(Get-ChildItem -LiteralPath $dist -Filter '*.dll' | ForEach-Object FullName)
    # the certificate behind a signature lives three days; the timestamp keeps the signature valid after that
    Invoke-Checked 'signing' { & $signtool sign /fd SHA256 /tr 'http://timestamp.acs.microsoft.com' /td SHA256 /dlib $dlib /dmdf $meta @signed | Out-Null }
    Invoke-Checked 'checking the signatures' { & $signtool verify /pa @signed | Out-Null }
    $sig = Get-AuthenticodeSignature $exe
    Write-Host "[package] signed: $($sig.Status), $($sig.SignerCertificate.Subject)"
}
$size = [math]::Round((Get-ChildItem $dist -File | Measure-Object Length -Sum).Sum / 1MB, 1)
Write-Host "[package] $dist ($size MB): $((Get-ChildItem $dist -File | ForEach-Object Name) -join ', ')"

$iss = @("/DVersion=$version")
if (-not $NoSign) {
    # inno signs the installer and its uninstaller with this command: $q is a quote to it and $f the file. single
    # quotes, so powershell leaves them alone
    $iss += '/DSigned', ('/Sartifact=$q' + $signtool + '$q sign /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 /dlib $q' + $dlib + '$q /dmdf $q' + $meta + '$q $f')
}
Invoke-Checked 'the installer' { & $iscc /Q @iss (Join-Path $PSScriptRoot 'installer.iss') }
$setup = Join-Path $repo "dist\eyepiece-$version-windows-setup.exe"
if (-not $NoSign) { Invoke-Checked 'checking the installer''s signature' { & $signtool verify /pa $setup | Out-Null } }
Write-Host "[package] $setup ($([math]::Round((Get-Item -LiteralPath $setup).Length / 1MB, 1)) MB)"

if ($Install) {
    $to = Join-Path $env:LOCALAPPDATA 'Programs\eyepiece'
    New-Item -ItemType Directory -Force $to | Out-Null
    Copy-Item (Join-Path $dist '*') $to -Force
    Write-Host "[package] installed to $to"
}
