# gate 6: does browsing a network share open nothing?
#   powershell -NoProfile -ExecutionPolicy Bypass -File spike\gate6.ps1                 this pc's smb client
#   powershell -NoProfile -ExecutionPolicy Bypass -File spike\gate6.ps1 -Mac user@host  a mac's smb client
# brings up a scratch samba share in its own small wsl distro (eyepiece-gate6, alpine), runs each mode of
# gate6_browse against a folder the client has never seen, and counts what the server logged.
# never touches a real share. needs wsl 2 with its default (nat) networking; build gate6_browse first
# (spike\gate.ps1 here, spike/mac/gate.sh on the mac). the mac reaches the share through an ssh tunnel.
param([string]$Mac, [string]$MacRepo = '~/src/eyepiece')
$ErrorActionPreference = 'Stop'
$env:WSL_UTF8 = '1'
$distro = 'eyepiece-gate6'
$store = Join-Path $env:LOCALAPPDATA 'eyepiece-gate6-wsl'
$exe = Join-Path $PSScriptRoot 'target\release\gate6_browse.exe'
$env:PATH = (Join-Path $PSScriptRoot 'third_party\ffmpeg\bin') + ';' + $env:PATH
# git's ssh: the one that ships with windows refuses the key file's permissions
$ssh = Join-Path $env:ProgramFiles 'Git\usr\bin\ssh.exe'
$mount = "$MacRepo/gate6-mnt"
if (-not $Mac -and -not (Test-Path $exe)) { throw 'build it first: spike\gate.ps1 gate6_browse list .' }

if (-not (@(wsl.exe -l -q) | Where-Object { $_.Trim() -eq $distro })) {
    New-Item -ItemType Directory $store -Force | Out-Null
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
function Invoke-Mac([string]$command) { & $ssh -o BatchMode=yes $Mac $command; if ($LASTEXITCODE -ne 0) { throw "on the mac, failed: $command" } }
function Measure-Match($lines, $pattern) { @($lines | Where-Object { $_ -match $pattern }).Count }

# open is the control: if it shows no opens, the log is not working and the rest means nothing.
# idle runs nothing: whatever is opened then, the client's own system opened. the last mode repeats a listing.
$verdictMode = if ($Mac) { 'bulk' } else { 'list' }
$modes = if ($Mac) { 'idle', 'open', 'names', 'bulk', 'stat', 'bulk' } else { 'idle', 'open', 'list', 'stat', 'attrs', 'list' }

# holds the distro up between the calls below; wsl may stop one that has nothing started from windows in it
Start-Process wsl.exe -ArgumentList '-d', $distro, '--', 'tail', '-f', '/dev/null' -WindowStyle Hidden
$tunnel = $null
$connected = $false
try {
    $samba = Invoke-Share setup $pw | Select-Object -Last 1
    if ((wsl.exe -d $distro -- ip -4 -o addr show eth0) -join ' ' -notmatch 'inet (\d+\.\d+\.\d+\.\d+)') { throw 'the distro has no address' }
    $ip = $Matches[1]
    $share = "\\$ip\gate6"
    $run = 'r' + (Get-Date -Format 'yyyyMMdd-HHmmss')
    $folders = @($modes | Where-Object { $_ -ne 'idle' } | Sort-Object -Unique)
    Invoke-Share fresh $run @folders
    if ($Mac) {
        # the mac cannot route to the distro: its port 4455 is carried over ssh to this pc and on to the share
        $tunnel = Start-Process $ssh -ArgumentList '-N', '-o', 'BatchMode=yes', '-o', 'ExitOnForwardFailure=yes', '-R', "4455:${ip}:445", $Mac -WindowStyle Hidden -PassThru
        Start-Sleep -Seconds 3
        Invoke-Mac "mkdir -p $mount && mount_smbfs -o nobrowse //gate6:$pw@127.0.0.1:4455/gate6 $mount"
        "share $share through an ssh tunnel, samba $samba, client macos $(& $ssh -o BatchMode=yes $Mac 'sw_vers -productVersion'), 20 files a folder"
    } else {
        net.exe use $share /user:gate6 $pw /persistent:no | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "could not connect to $share" }
        "share $share, samba $samba, client windows $([Environment]::OSVersion.Version), 20 files a folder"
    }
    $connected = $true
    $log = '/var/log/samba/smbd.log'
    $kinds = 'access', 'modify', 'change'

    $rows = foreach ($mode in $modes) {
        # clients keep directory and file information for a while; wait it out
        Start-Sleep -Seconds 15
        $before = [int](wsl.exe -d $distro -- sh -c "wc -l < $log")
        $changed = @()
        if ($mode -eq 'idle') {
            Start-Sleep -Seconds 20
        } else {
            $t0 = (Invoke-Share times "$run-$mode").Trim() -split ' '
            if ($Mac) { Invoke-Mac "$MacRepo/spike/mac/target/release/gate6_browse $mode $mount/$run-$mode" | Out-Null }
            else {
                & $exe $mode "$share\$run-$mode" | Out-Null
                if ($LASTEXITCODE -ne 0) { throw "gate6_browse $mode failed" }
            }
            Start-Sleep -Seconds 2
            $t1 = (Invoke-Share times "$run-$mode").Trim() -split ' '
            $changed = @(0..2 | Where-Object { $t0[$_] -ne $t1[$_] } | ForEach-Object { $kinds[$_] })
        }
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
    $control = $rows | Where-Object { $_.mode -eq 'open' } | Select-Object -First 1
    $listing = $rows | Where-Object { $_.mode -eq $verdictMode } | Select-Object -First 1
    if ($control.'files opened' -ne 20 -or $control.'read requests' -eq 0) { 'FAIL the control shows no opens: the server log is not working'; exit 1 }
    if ($listing.'files opened' -eq 0 -and $listing.'read requests' -eq 0 -and $listing.'times changed' -eq 'none') {
        "PASS listing a folder ($verdictMode) opened no file in it"
    } else { "FAIL listing a folder ($verdictMode) opened files"; exit 1 }
} finally {
    if ($connected -and $Mac) { & $ssh -o BatchMode=yes $Mac "umount $mount" }
    if ($connected -and -not $Mac) { net.exe use $share /delete /y 2>$null | Out-Null }
    if ($tunnel) { Stop-Process -Id $tunnel.Id -ErrorAction SilentlyContinue }
    # stops this distro only, and samba and the keep-alive with it
    wsl.exe -t $distro | Out-Null
}
