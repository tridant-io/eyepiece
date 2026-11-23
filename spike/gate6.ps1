# gate 6: does browsing a network share open nothing?
#   powershell -NoProfile -ExecutionPolicy Bypass -File spike\gate6.ps1
# brings up a scratch samba share in its own small wsl distro (eyepiece-gate6, alpine), runs each mode of
# gate6_browse against a folder the windows client has never seen, and counts what the server logged.
# never touches a real share. needs wsl 2 with its default (nat) networking; build gate6_browse first.
$ErrorActionPreference = 'Stop'
$env:WSL_UTF8 = '1'
$distro = 'eyepiece-gate6'
$store = Join-Path $env:LOCALAPPDATA 'eyepiece-gate6-wsl'
$exe = Join-Path $PSScriptRoot 'target\release\gate6_browse.exe'
$env:PATH = (Join-Path $PSScriptRoot 'third_party\ffmpeg\bin') + ';' + $env:PATH
if (-not (Test-Path $exe)) { throw 'build it first: spike\gate.ps1 gate6_browse list .' }

if (-not (@(wsl.exe -l -q) | Where-Object { $_.Trim() -eq $distro })) {
    New-Item -ItemType Directory $store | Out-Null
    $tar = Join-Path $store 'alpine-minirootfs.tar.gz'
    Invoke-WebRequest 'https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/x86_64/alpine-minirootfs-3.24.4-x86_64.tar.gz' -OutFile $tar
    if ((Get-FileHash $tar -Algorithm SHA256).Hash -ne 'CDAA38CB8DE3537967EE5B6C9B6CD00CC962DA2B018E6EEA48696BB274923012') { throw 'the alpine download does not match its checksum' }
    wsl.exe --import $distro $store $tar --version 2 | Out-Null
}
$pwFile = Join-Path $store 'share-password.txt'
if (-not (Test-Path $pwFile)) {
    Set-Content $pwFile (-join ((48..57) + (97..122) | Get-Random -Count 20 | ForEach-Object { [char]$_ })) -Encoding ascii -NoNewline
}
$pw = Get-Content $pwFile -Raw

# the script is piped in with carriage returns stripped, since git may check it out with windows line ends
$sh = '/mnt/' + $PSScriptRoot.Substring(0, 1).ToLower() + $PSScriptRoot.Substring(2).Replace('\', '/') + '/gate6-share.sh'
function Invoke-Share { wsl.exe -d $distro -- sh -c "tr -d '\r' < '$sh' | sh -s -- $args" }
function Measure-Match($lines, $pattern) { @($lines | Where-Object { $_ -match $pattern }).Count }

# wsl stops a distro once nothing started from windows is running in it, and samba with it
$keep = Start-Process wsl.exe -ArgumentList '-d', $distro, '--', 'tail', '-f', '/dev/null' -WindowStyle Hidden -PassThru
try {
    $samba = Invoke-Share setup $pw | Select-Object -Last 1
    if ((wsl.exe -d $distro -- ip -4 -o addr show eth0) -join ' ' -notmatch 'inet (\d+\.\d+\.\d+\.\d+)') { throw 'the distro has no address' }
    $share = "\\$($Matches[1])\gate6"
    $run = 'r' + (Get-Date -Format 'yyyyMMdd-HHmmss')
    Invoke-Share fresh $run
    "share $share, samba $samba, windows $([Environment]::OSVersion.Version), 20 files a folder"
    net.exe use $share /user:gate6 $pw /persistent:no | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not connect to $share" }
    $log = '/var/log/samba/smbd.log'
    $kinds = 'access', 'modify', 'change'

    # open is the control and runs first: if it shows no opens, the log is not working and the rest means nothing.
    # the last run lists the same folder a second time.
    $rows = foreach ($mode in 'open', 'list', 'stat', 'attrs', 'list') {
        # windows keeps directory and file information for about 10 s; wait it out
        Start-Sleep -Seconds 15
        $before = [int](wsl.exe -d $distro -- sh -c "wc -l < $log")
        $t0 = (Invoke-Share times "$run-$mode").Trim() -split ' '
        & $exe $mode "$share\$run-$mode" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "gate6_browse $mode failed" }
        Start-Sleep -Seconds 2
        $t1 = (Invoke-Share times "$run-$mode").Trim() -split ' '
        $changed = @(0..2 | Where-Object { $t0[$_] -ne $t1[$_] } | ForEach-Object { $kinds[$_] })
        $new = @(wsl.exe -d $distro -- tail -n "+$($before + 1)" $log)
        $fileOpens = @($new | Where-Object { $_ -match '\|create_file\|.*clip_\d\d\.mov' })
        [pscustomobject]@{
            mode            = $mode
            'open requests' = Measure-Match $new 'opcode\[SMB2_OP_CREATE\]'
            'list requests' = Measure-Match $new 'opcode\[SMB2_OP_QUERY_DIRECTORY\]'
            'read requests' = Measure-Match $new 'opcode\[SMB2_OP_READ\]'
            'files opened'  = @($fileOpens | ForEach-Object { if ($_ -match '(clip_\d\d\.mov)') { $Matches[1] } } | Sort-Object -Unique).Count
            'access asked'  = (@($fileOpens | ForEach-Object { if ($_ -match '\|(0x[0-9a-fA-F]+)\|') { $Matches[1] } } | Group-Object | ForEach-Object { "$($_.Name) x$($_.Count)" }) -join ', ')
            'times changed' = if ($changed) { $changed -join ', ' } else { 'none' }
        }
    }
    $rows | Format-Table -AutoSize | Out-String -Width 200
    $control = $rows[0]
    $list = $rows[1]
    if ($control.'files opened' -ne 20 -or $control.'read requests' -eq 0) { 'FAIL the control shows no opens: the server log is not working'; exit 1 }
    if ($list.'files opened' -eq 0 -and $list.'read requests' -eq 0 -and $list.'open requests' -le 1 -and $list.'times changed' -eq 'none') {
        'PASS listing a folder opened the folder and no file in it'
    } else { 'FAIL listing a folder opened files'; exit 1 }
} finally {
    if ($share) { net.exe use $share /delete /y 2>$null | Out-Null }
    Stop-Process -Id $keep.Id -ErrorAction SilentlyContinue
}
