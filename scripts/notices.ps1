# writes THIRD-PARTY-NOTICES.txt for the windows release build: every crate compiled into eyepiece.exe with its license
# and the text of it, ffmpeg, and the two typefaces. the same lock file, crates and ffmpeg always give the same bytes.
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\notices.ps1          writes dist\eyepiece\THIRD-PARTY-NOTICES.txt
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\notices.ps1 -Check   fails, naming the crate, on a license off the allow list
# the texts come from each crate's own source in the local cargo registry, so the crates must have been fetched (any
# cargo build does that). scripts\package.ps1 runs it.
param([string]$Out, [string]$Ffmpeg, [switch]$Check)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$target = 'x86_64-pc-windows-msvc'
if (-not $Out) { $Out = Join-Path $repo 'dist\eyepiece\THIRD-PARTY-NOTICES.txt' }
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)

# the licenses a crate may be under. where a crate offers a choice, the first of these it offers is the one taken.
$allowed = 'MIT', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'ISC', 'Zlib', 'Unicode-3.0', 'Unicode-DFS-2016', 'BSL-1.0', 'CC0-1.0', 'MPL-2.0', 'OFL-1.1', 'Unlicense', '0BSD'
$rank = @{}
for ($i = 0; $i -lt $allowed.Count; $i++) { $rank[$allowed[$i]] = $i }
$off = [int]::MaxValue

function Invoke-Cargo {
    # 'Continue' for the call only: powershell 5.1 turns a native command's stderr into errors. the exit code is the verdict.
    $ErrorActionPreference = 'Continue'
    $lines = & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo $($args -join ' ') failed (exit $LASTEXITCODE)" }
    $lines
}

function Sort-Ordinal([object[]]$Items, [string[]]$Keys) {
    # sort-object follows the culture and ignores case; the file must not depend on either. the casts pick the overload
    # that sorts $Items in place: the generic one sorts a copy.
    [Array]::Sort([Array]$Keys, [Array]$Items, [Collections.IComparer][StringComparer]::Ordinal)
    $Items
}

# an spdx expression, read into the licenses taken and the place of the worst of them on the allow list ($off if one is
# not on it). OR takes the branch highest on the list, AND takes every part. cargo still accepts "a/b" for "a OR b".
function Read-License([string]$Text) {
    $script:tokens = @(($Text -replace '/', ' OR ' -replace '([()])', ' $1 ') -split '\s+' | Where-Object { $_ })
    $script:at = 0
    $result = Read-Any
    if ($script:at -lt $script:tokens.Count) { throw "cannot read the license expression '$Text'" }
    $result
}
function Read-Any {
    $best = Read-All
    while ($script:tokens[$script:at] -ceq 'OR') {
        $script:at++
        $next = Read-All
        if ($next.Rank -lt $best.Rank) { $best = $next }
    }
    $best
}
function Read-All {
    $all = Read-One
    while ($script:tokens[$script:at] -ceq 'AND') {
        $script:at++
        $next = Read-One
        $all = @{ Take = @($all.Take) + $next.Take; Rank = [math]::Max($all.Rank, $next.Rank) }
    }
    $all
}
function Read-One {
    $token = $script:tokens[$script:at]
    $script:at++
    if ($token -eq '(') {
        $inner = Read-Any
        if ($script:tokens[$script:at] -ne ')') { throw 'a ( without its ) in a license expression' }
        $script:at++
        return $inner
    }
    if (-not $token -or $token -in 'AND', 'OR', 'WITH', ')') { throw "cannot read a license expression at '$token'" }
    $place = if ($rank.ContainsKey($token)) { $rank[$token] } else { $off }
    @{ Take = @($token); Rank = $place }
}

$manifest = Join-Path $repo 'Cargo.toml'
$meta = (Invoke-Cargo metadata --quiet --locked --format-version 1 --filter-platform $target --manifest-path $manifest) -join "`n" | ConvertFrom-Json
$program = $meta.packages | Where-Object { $_.id -eq $meta.resolve.root }
$byName = @{}
foreach ($p in $meta.packages) { $byName["$($p.name) v$($p.version)"] += @($p) }

# what is linked into the program: its normal dependencies on windows. build scripts, what they use (bindgen, cc) and
# proc macros only run while it compiles. the tree's first line is the program itself.
$tree = Invoke-Cargo tree --quiet --locked --target $target --edges 'normal,no-proc-macro' --prefix none --format '{p}' --manifest-path $manifest
$seen = @{}
$crates = foreach ($line in $tree | Select-Object -Skip 1) {
    $id = $line -replace ' \(\*\)$', ''
    if ($seen[$id]) { continue }
    $seen[$id] = $true
    $found = $byName[$id]
    # a path or git dependency prints more than its name and version: it would need describing here by hand
    if (-not $found -or $found.Count -ne 1 -or $found[0].source -ne 'registry+https://github.com/rust-lang/crates.io-index') { throw "not a crates.io crate: '$id'" }
    $p = $found[0]
    if (-not $p.license) { throw "$($p.name) $($p.version) names no license in its manifest" }
    $license = Read-License $p.license
    $repository = if ($p.repository) { $p.repository } else { "https://crates.io/crates/$($p.name)/$($p.version)" }
    [pscustomobject]@{ Name = $p.name; Version = $p.version; Who = "$($p.name) $($p.version)"; Expression = $p.license
        Take = $license.Take; Allowed = $license.Rank -ne $off; Dir = Split-Path $p.manifest_path -Parent; Repository = $repository }
}
$crates = @(Sort-Ordinal $crates @($crates | ForEach-Object { $_.Who }))

$counts = @{}
foreach ($c in $crates) { foreach ($t in $c.Take) { $counts[$t] += 1 } }
$names = @($counts.Keys)
$taken = (Sort-Ordinal $names @($names | ForEach-Object { '{0:D6} {1}' -f (999999 - $counts[$_]), $_ }) | ForEach-Object { "$_ $($counts[$_])" }) -join ', '
Write-Host "[notices] $($crates.Count) crates compiled in, by the license taken: $taken"
$refused = @($crates | Where-Object { -not $_.Allowed })
if ($Check) {
    if ($refused.Count) { throw "not on the allow list: $(($refused | ForEach-Object { "$($_.Who) ($($_.Expression))" }) -join ', ')" }
    Write-Host '[notices] every crate is on the allow list'
    return
}

# one copy of each license text, grouped on its words with the copyright lines taken out; those are kept per part
$texts = New-Object 'System.Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal)
# the copyright sign is built from its code: powershell 5.1 reads a script without a byte order mark as ansi
$copyrightLine = '^\s*Copyright\s+(\([cC]\)|' + [char]0xA9 + '|\[?\d)'
function Add-Text([string]$Who, [string]$Label, [string]$Path) {
    $copyright = New-Object 'System.Collections.Generic.List[string]'
    $body = New-Object 'System.Collections.Generic.List[string]'
    foreach ($line in [IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) -split '\r?\n') {
        # only "Copyright (c)", a copyright sign or a year: the ofl's wrapped "Copyright Holder(s) ..." and apache's
        # "Copyright [yyyy] [name of copyright owner]" are part of the text. a notice in another form stays in the text.
        if ($line -cmatch $copyrightLine) { $copyright.Add($line.Trim()) } else { $body.Add($line.TrimEnd()) }
    }
    while ($body.Count -and -not $body[0]) { $body.RemoveAt(0) }
    while ($body.Count -and -not $body[$body.Count - 1]) { $body.RemoveAt($body.Count - 1) }
    $key = (($body -join ' ') -replace '\s+', ' ').Trim()
    if (-not $texts.ContainsKey($key)) { $texts[$key] = [pscustomobject]@{ Body = $body; Labels = @{}; Parts = @{} } }
    $group = $texts[$key]
    $group.Labels[$Label] = $true
    if (-not $group.Parts.ContainsKey($Who)) { $group.Parts[$Who] = New-Object 'System.Collections.Generic.List[string]' }
    foreach ($line in $copyright) { if (-not $group.Parts[$Who].Contains($line)) { $group.Parts[$Who].Add($line) } }
}

# a crate's license files, at any depth (signalsmith-stretch carries the c++ it builds, with that code's own licenses).
# LICENSE-MIT and the like name one license and are kept only when it is the one taken; LICENSE, COPYING and NOTICE
# files name none and are always kept.
$bare = @()
foreach ($c in $crates) {
    $any = $false
    $kept = @(foreach ($f in Get-ChildItem -LiteralPath $c.Dir -Recurse -File) {
        if ($f.Name -notmatch '^(licen[cs]e|copying|notice)(-(?<tag>[\w.-]+?))?(\.md|\.txt)?$') { continue }
        $any = $true
        $tag = $Matches['tag']
        $label = if ($tag) { @($c.Take | Where-Object { $_.StartsWith($tag, [StringComparison]::OrdinalIgnoreCase) })[0] } else { $c.Take -join ' AND ' }
        if (-not $label) { continue }
        [pscustomobject]@{ Rel = $f.FullName.Substring($c.Dir.Length + 1).Replace('\', '/'); Label = $label; Path = $f.FullName }
    })
    if (-not $kept.Count) {
        if ($any) { throw "$($c.Who) ships license files, but none for $($c.Take -join ' AND '), the license taken" }
        $bare += $c
        continue
    }
    if ($c.Name -eq 'signalsmith-stretch') {
        foreach ($need in 'signalsmith-stretch/LICENSE.txt', 'signalsmith-linear/LICENSE.txt') {
            if ($need -notin $kept.Rel) { throw "$($c.Who) no longer has $need, the license of the c++ it builds in" }
        }
    }
    foreach ($k in Sort-Ordinal $kept @($kept | ForEach-Object { $_.Rel })) {
        $who = if ($k.Rel.Contains('/')) { "$($c.Who) ($($k.Rel))" } else { $c.Who }
        Add-Text $who $k.Label $k.Path
    }
}
$fonts = Join-Path $repo 'assets\fonts'
Add-Text 'IBM Plex Mono' 'OFL-1.1' (Join-Path $fonts 'IBM-Plex-LICENSE.txt')
Add-Text 'Archivo' 'OFL-1.1' (Join-Path $fonts 'Archivo-OFL.txt')

# ffmpeg, as the libraries themselves say: one build, its license and its configuration. by default the one the build
# links against: FFMPEG_DIR, else what scripts\setup.ps1 fetched.
if (-not $Ffmpeg) { $Ffmpeg = if ($env:FFMPEG_DIR) { $env:FFMPEG_DIR } else { Join-Path $repo 'third_party\ffmpeg' } }
if (-not (Test-Path (Join-Path $Ffmpeg 'bin\avutil-*.dll'))) { throw "no ffmpeg in $Ffmpeg`: run scripts\setup.ps1 first" }
$versions = @(Get-ChildItem (Join-Path $Ffmpeg 'bin') -Filter '*.dll' | ForEach-Object { $_.VersionInfo.ProductVersion } | Select-Object -Unique)
if ($versions.Count -ne 1) { throw "the ffmpeg libraries are not one build: $($versions -join ', ')" }
$ffVersion = $versions[0]
$avutil = Get-Item (Join-Path $Ffmpeg 'bin\avutil-*.dll')
$raw = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($avutil.FullName))
$ffLicense = [regex]::Match($raw, '(L?GPL version [\d.]+ or later|nonfree and unredistributable)').Value
if ($ffLicense -notlike 'LGPL *') { throw "this ffmpeg is '$ffLicense', not lgpl, and must not ship (adr 0005)" }
$at = $raw.IndexOf('--enable-shared')
if ($at -lt 0) { throw 'this ffmpeg was not built as shared libraries, and must not ship (adr 0005)' }
$from = $raw.LastIndexOf([char]0, $at) + 1
$config = $raw.Substring($from, $raw.IndexOf([char]0, $at) - $from)
$download = [regex]::Match([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'setup.ps1')), 'https://github\.com/BtbN/FFmpeg-Builds/[^''"\s]+').Value
if (-not $download) { throw 'scripts\setup.ps1 no longer says where ffmpeg comes from' }
$revision = if ($ffVersion -match '-g([0-9a-f]+)') { "commit $($Matches[1])" } else { "tag $ffVersion" }

$rule = '=' * 100
$doc = New-Object 'System.Collections.Generic.List[string]'
function Add-Line([string[]]$Lines) { foreach ($l in $Lines) { $doc.Add($l) } }
Add-Line "third-party notices: eyepiece $($program.version) for windows x64", '',
    'eyepiece contains open source software made by others. each part is under its own license:', '',
    "  1. ffmpeg, the separate av*.dll and sw*.dll files beside eyepiece.exe: $ffLicense",
    "  2. $($crates.Count) rust crates built into eyepiece.exe: $taken",
    '  3. two typefaces built into eyepiece.exe: OFL-1.1', '',
    'section 4 gives each license text once, worded as the parts ship it, with the copyright lines of every part',
    'it covers.', ''

Add-Line $rule, '1. ffmpeg', '',
    "eyepiece reads and decodes video with ffmpeg (https://ffmpeg.org). $($avutil.VersionInfo.LegalCopyright).",
    "license: $ffLicense, as the build itself says. the text is ffmpeg-LICENSE.txt beside this file.",
    "version: $ffVersion, the windows build made by https://github.com/BtbN/FFmpeg-Builds,",
    "downloaded from $download",
    "source: https://git.ffmpeg.org/ffmpeg.git at $revision. the scripts that made the build, and the source of",
    'each library it includes, are at https://github.com/BtbN/FFmpeg-Builds.', '',
    'ffmpeg is not part of eyepiece.exe. it is the separate files av*.dll and sw*.dll beside it, which eyepiece loads',
    'from its own folder when it starts. you may replace them with ffmpeg built your own way, changed or not: build it',
    'as shared libraries with the same major versions (the number in each file name) and put your files in place of',
    'these. eyepiece needs nothing else to use them.', '',
    'how this build of ffmpeg was configured:'
$line = ''
foreach ($word in $config -split ' ') {
    if ($line -and $line.Length + 1 + $word.Length -gt 98) { Add-Line "  $line"; $line = $word } else { $line = "$line $word".Trim() }
}
Add-Line "  $line", ''

$wide = ($crates | ForEach-Object { $_.Who.Length } | Measure-Object -Maximum).Maximum + 2
$wideExpr = ($crates | ForEach-Object { $_.Expression.Length } | Measure-Object -Maximum).Maximum + 2
$wideTake = ($crates | ForEach-Object { ($_.Take -join ' AND ').Length } | Measure-Object -Maximum).Maximum + 2
Add-Line $rule, '2. rust crates', '',
    "the $($crates.Count) crates built into eyepiece.exe, each from crates.io at the version shown. where a crate offers a",
    'choice of licenses, eyepiece takes the first it offers of these:',
    "  $($allowed -join ', ')",
    'section 4 has the text of the license taken, as the crate ships it, and any license file of the crate that does not',
    'name one license.', '',
    ('  ' + 'crate'.PadRight($wide) + 'license'.PadRight($wideExpr) + 'taken'.PadRight($wideTake) + 'repository')
foreach ($c in $crates) { Add-Line ('  ' + $c.Who.PadRight($wide) + $c.Expression.PadRight($wideExpr) + ($c.Take -join ' AND ').PadRight($wideTake) + $c.Repository) }
Add-Line '', 'signalsmith-stretch also builds in two c++ libraries by signalsmith audio, signalsmith-stretch and',
    'signalsmith-linear (https://github.com/Signalsmith-Audio), each under the MIT license with its own copyright line.', ''
if ($bare.Count) {
    Add-Line 'these crates ship no license file in their published source. their license is the one their manifest names,',
        'shown above; where section 4 has a text of that license, it is the text that applies.'
    foreach ($c in $bare) { Add-Line "  $($c.Who)" }
    Add-Line ''
}

Add-Line $rule, '3. typefaces', '',
    'the controls are drawn in ibm plex mono and archivo, which are built into eyepiece.exe. both are under the SIL Open',
    'Font License 1.1: the text is in section 4, and in the fonts-*.txt files beside this file.', ''

foreach ($g in $texts.Values) {
    $g | Add-Member Label ((Sort-Ordinal @($g.Labels.Keys) @($g.Labels.Keys)) -join ', ')
    $g | Add-Member Who @(Sort-Ordinal @($g.Parts.Keys) @($g.Parts.Keys))
}
$groups = @($texts.Values)
$groups = @(Sort-Ordinal $groups @($groups | ForEach-Object { "$($_.Label)`t$($_.Who[0])" }))
Add-Line $rule, '4. license texts', ''
foreach ($g in $groups) {
    Add-Line ('-' * 100), $g.Label, '', 'covers:'
    foreach ($who in $g.Who) {
        Add-Line "  $who"
        foreach ($l in $g.Parts[$who]) { Add-Line "      $l" }
    }
    Add-Line (@('') + @($g.Body) + @(''))
}

New-Item -ItemType Directory -Force (Split-Path $Out -Parent) | Out-Null
[IO.File]::WriteAllText($Out, ($doc -join "`r`n") + "`r`n", (New-Object Text.UTF8Encoding $false))
Write-Host "[notices] no license file: $(($bare | ForEach-Object { $_.Who }) -join ', ')"
Write-Host "[notices] wrote $Out ($([math]::Round((Get-Item $Out).Length / 1KB)) KB, $($groups.Count) texts)"
