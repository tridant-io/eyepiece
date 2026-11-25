# writes eyepiece-release.txt, the release manifest the player's update reads (src\update, adr 0007): the version, the
# date, a line for each change in the notes, and every file of dist\eyepiece with its size and sha-256. with -Publish it
# then makes the release v<version> on github with gh: the files and the manifest as its assets, the notes as its text.
# run scripts\package.ps1 first: it builds, signs and fills dist\eyepiece.
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release.ps1 -Notes notes.md            writes dist\eyepiece-release.txt
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release.ps1 -Notes notes.md -Publish   and makes the release
# the notes are markdown, as tridant's release notes are written: a heading for each kind of change (added, changed,
# fixed) and a bullet under it for each change. links and bold stay on the release page; the player shows the words.
# -Dist and -Out read and write elsewhere, to try it on a copy. nothing is deleted.
param([Parameter(Mandatory)][string]$Notes, [string]$Dist, [string]$Out, [switch]$Publish)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
if (-not $Dist) { $Dist = Join-Path $repo 'dist\eyepiece' }
$Dist = (Resolve-Path -LiteralPath $Dist).Path
if (-not $Out) { $Out = Join-Path (Split-Path $Dist -Parent) 'eyepiece-release.txt' }
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
# inside the folder it would be listed as a file of the release the next time
if ((Split-Path $Out -Parent).TrimEnd('\') -eq $Dist.TrimEnd('\')) { throw "write the manifest beside $Dist, not in it" }
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

# in ordinal order, so the same folder always gives the same bytes. the names are sorted, not the files: a sort of
# the listing's items sorts a copy of them
$names = [string[]]@(Get-ChildItem -LiteralPath $Dist -File | ForEach-Object Name)
[Array]::Sort($names, [StringComparer]::Ordinal)
$files = @($names | ForEach-Object { Get-Item -LiteralPath (Join-Path $Dist $_) })
foreach ($f in $files) {
    # the names the player takes from a release and no other (plain, in src\update\mod.rs)
    if ($f.Name -cnotmatch '^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$' -or $f.Name -notmatch '\.(exe|dll|txt)$' -or $f.Name -match '\.old-' -or $f.Name -match '^(con|prn|aux|nul|com\d|lpt\d)(\.|$)') {
        throw "the player takes no file named $($f.Name) from a release"
    }
}
$exe = Join-Path $Dist 'eyepiece.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "no eyepiece.exe in $Dist`: run scripts\package.ps1 first" }
# the program's user agent carries its version: a program built from another version is caught here. so is one whose
# player never calls src\update, as the build leaves out code nothing calls
if ([Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($exe)).IndexOf("eyepiece/$version") -lt 0) {
    throw "$exe does not say it is eyepiece $version (eyepiece/$version is not in it): it was built from another version, so run scripts\package.ps1 again, or its player does not call src\update yet"
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

# the invariant culture's calendar: another culture's could give another year
$lines = @("version = $version", "date = $([DateTime]::Now.ToString('yyyy-MM-dd', [Globalization.CultureInfo]::InvariantCulture))") + $changes
foreach ($f in $files) {
    $sum = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $lines += "file = $($f.Name) $dot $($f.Length) $dot $sum"
}
New-Item -ItemType Directory -Force (Split-Path $Out -Parent) | Out-Null
[IO.File]::WriteAllText($Out, ($lines -join "`n") + "`n", (New-Object Text.UTF8Encoding $false))
Write-Host "[release] wrote $Out`: eyepiece $version, $($files.Count) files, $($changes.Count) changes"

if ($Publish) {
    if ((Split-Path $Out -Leaf) -cne 'eyepiece-release.txt') { throw 'the player asks for its manifest as eyepiece-release.txt: publish it under that name' }
    # the update takes a program or library only when it is signed as the running program is
    $signers = @(foreach ($f in $files | Where-Object { $_.Extension -in '.exe', '.dll' }) {
        $signature = Get-AuthenticodeSignature -LiteralPath $f.FullName
        if ($signature.Status -ne 'Valid') { throw "$($f.Name) is not signed ($($signature.Status)): run scripts\package.ps1 with signing" }
        $signature.SignerCertificate.Subject
    })
    $who = @($signers | Select-Object -Unique)
    if ($who.Count -ne 1) { throw "the files are signed by more than one signer: $($who -join '; ')" }
    $commit = git -C $repo rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'git cannot name the commit this was built from' }
    Invoke-Checked 'gh release create' { gh release create "v$version" @($files | ForEach-Object FullName) $Out --repo tridant-io/eyepiece --title "eyepiece $version" --notes-file $Notes --target $commit }
    Write-Host "[release] published v$version, signed by $($who[0])"
}
