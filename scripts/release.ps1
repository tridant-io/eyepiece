# writes eyepiece-release.txt, the release manifest the player's update reads (src\update, adr 0007): the version, the
# date, a line for each change in the notes, and the windows installer with its size and sha-256 (adr 0008). with
# -Publish it then makes the release v<version> on github with gh: the installer and the manifest as its assets, with
# -Mac's disk image as it is, kept out of the manifest, and the notes as its text.
# run scripts\package.ps1 first: it builds, signs and makes dist\eyepiece-<version>-windows-setup.exe.
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release.ps1 -Notes notes.md                          writes dist\eyepiece-release.txt
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release.ps1 -Notes notes.md -Publish [-Mac x.dmg]    and makes the release
# the notes are markdown, as tridant's release notes are written: a heading for each kind of change (added, changed,
# fixed) and a bullet under it for each change. links and bold stay on the release page; the player shows the words.
# -Dist and -Out read and write elsewhere, to try it on a copy. nothing is deleted.
param([Parameter(Mandatory)][string]$Notes, [string]$Dist, [string]$Out, [string]$Mac, [switch]$Publish)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
if (-not $Dist) { $Dist = Join-Path $repo 'dist' }
$Dist = (Resolve-Path -LiteralPath $Dist).Path
if (-not $Out) { $Out = Join-Path $Dist 'eyepiece-release.txt' }
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
# the manifest's separator, built from its code: powershell 5.1 reads a script without a byte order mark as ansi
$dot = [char]0xB7

function Invoke-Checked {
    param([string]$What, [scriptblock]$Run)
    # 'Continue' for the call only: powershell 5.1 turns a native command's stderr into errors. the exit code is the verdict.
    $ErrorActionPreference = 'Continue'
    & $Run
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

if ([IO.File]::ReadAllText((Join-Path $repo 'Cargo.toml')) -notmatch '(?m)^version = "(\d+\.\d+\.\d+)"') { throw 'Cargo.toml names no version x.y.z' }
$version = $Matches[1]
# the one file of a release the player takes, by the name it asks for (setup_name in src\update\mod.rs)
$setup = Join-Path $Dist "eyepiece-$version-windows-setup.exe"
if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) { throw "no eyepiece-$version-windows-setup.exe in $Dist`: run scripts\package.ps1 first" }
$setup = Get-Item -LiteralPath $setup
if ($Mac) {
    $Mac = (Resolve-Path -LiteralPath $Mac).Path
    if ($Mac -notmatch '\.dmg$') { throw "-Mac takes the mac's disk image, a .dmg: $Mac" }
}

$kind = ''
$changes = @(foreach ($line in [IO.File]::ReadAllLines((Resolve-Path -LiteralPath $Notes).Path, [Text.Encoding]::UTF8)) {
    if ($line -match '^\s*#+\s*(.+?)\s*$') { $kind = $Matches[1].ToLowerInvariant(); continue }
    if ($line -match '^\s*[-*]\s+(.+?)\s*$') {
        if (-not $kind) { throw "a change in $Notes comes before any heading: $line" }
        $words = $Matches[1] -replace '\[([^\]]*)\]\([^)]*\)', '$1' -replace '\*\*|__|`', ''
        "notes = $kind $dot $words"
    }
})
if (-not $changes.Count) { throw "$Notes has no changes in it: a heading for each kind, and a bullet under it for each change" }

$sum = (Get-FileHash -LiteralPath $setup.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
# the invariant culture's calendar: another culture's could give another year
$lines = @("version = $version", "date = $([DateTime]::Now.ToString('yyyy-MM-dd', [Globalization.CultureInfo]::InvariantCulture))") + $changes + "file = $($setup.Name) $dot $($setup.Length) $dot $sum"
New-Item -ItemType Directory -Force (Split-Path $Out -Parent) | Out-Null
[IO.File]::WriteAllText($Out, ($lines -join "`n") + "`n", (New-Object Text.UTF8Encoding $false))
Write-Host "[release] wrote $Out`: eyepiece $version, $($setup.Name), $($changes.Count) changes"

if ($Publish) {
    if ((Split-Path $Out -Leaf) -cne 'eyepiece-release.txt') { throw 'the player asks for its manifest as eyepiece-release.txt: publish it under that name' }
    # the update runs an installer only when it is signed as the running program is
    $signature = Get-AuthenticodeSignature -LiteralPath $setup.FullName
    if ($signature.Status -ne 'Valid') { throw "$($setup.Name) is not signed ($($signature.Status)): run scripts\package.ps1 with signing" }
    $commit = git -C $repo rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'git cannot name the commit this was built from' }
    $assets = @($setup.FullName, $Out)
    if ($Mac) { $assets += $Mac }
    Invoke-Checked 'gh release create' { gh release create "v$version" @assets --repo tridant-io/eyepiece --title "eyepiece $version" --notes-file $Notes --target $commit }
    Write-Host "[release] published v$version`: $(($assets | Split-Path -Leaf) -join ', '), signed by $($signature.SignerCertificate.Subject)"
}
