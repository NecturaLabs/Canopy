# canopy-installer (this marker line identifies the script; keep it)
#
# Installs, upgrades or removes Canopy for the current Windows user, without administrator rights:
#
#   irm https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.ps1 | iex
#
# With options (here: uninstall), run the downloaded text as a script block:
#
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.ps1))) -Uninstall
#
# The script reads the update manifest in NecturaLabs/app-updates (canopy/latest.json, or
# canopy/beta.json), downloads the Windows archive over HTTPS, checks its SHA-256 against the
# manifest and refuses anything that does not match. It never runs what it downloads and never
# installs git.
#
# Layout: everything lives in %LOCALAPPDATA%\Programs\Canopy (CANOPY_INSTALL_DIR overrides it):
#   canopy.exe  README.md  LICENSE  legal\  VERSION  uninstall.ps1  .canopy-install (receipt)
# Outside it, only: a Start Menu shortcut, an "Apps & features" entry under
# HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.necturalabs.Canopy, and the folder
# on the user PATH (-NoPath skips it).
#
# Works in Windows PowerShell 5.1 and PowerShell 7. The source is packaging/install.ps1 in
# NecturaLabs/Canopy; the release workflow publishes it to NecturaLabs/app-updates/canopy/.
param(
    # Install the newest build, including pre-releases (the beta channel).
    [switch]$Beta,
    # Install this version (for example 0.2.0) instead of the newest one.
    [string]$Version = '',
    # Do not add the install folder to the user PATH.
    [switch]$NoPath,
    # Remove Canopy: exactly what this script created. Settings and data stay.
    [switch]$Uninstall,
    # With -Uninstall: also delete Canopy's settings, logs and caches (asks first).
    [switch]$Purge,
    # Do not ask for confirmation (for -Purge).
    [switch]$Yes,
    # Show the help.
    [switch]$Help
)

# Anything else on the command line (a mistyped option) lands here and is refused.
$ExtraArgs = @($args)
# The script file when run as one (-File, or the installed uninstall.ps1); empty when the text was
# piped into iex or run as a script block, where `exit` would close the user's PowerShell.
$ScriptFile = ''
if ($MyInvocation.MyCommand.CommandType -eq 'ExternalScript') { $ScriptFile = $MyInvocation.MyCommand.Path }

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 renders a progress bar per downloaded chunk, which slows downloads badly.
$ProgressPreference = 'SilentlyContinue'

$AppId = 'io.necturalabs.Canopy'
$FeedUrl = 'https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy'
$ReleasesUrl = 'https://github.com/NecturaLabs/app-updates/releases/download'
$UninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$AppId"
$InsecureLocal = $false

# -----------------------------------------------------------------------------------------------
# Canopy's user data, removed only by -Purge. Keep this list in step with the app (and with
# packaging/install.sh). Old (eframe::storage_dir) and current vendor-specific locations.
function Get-PurgeTargets {
    @(
        (Join-Path $env:APPDATA 'canopy'),
        (Join-Path $env:APPDATA 'NecturaLabs\Canopy'),
        (Join-Path $env:LOCALAPPDATA 'NecturaLabs\Canopy')
    )
}
# -----------------------------------------------------------------------------------------------

# The installer's own errors are ApplicationExceptions, which nothing else here throws, so they
# can be told apart from unexpected failures.
function Fail([string]$Message) { throw (New-Object System.ApplicationException $Message) }
function Say([string]$Message = '') { Write-Host $Message }
function Warn([string]$Message) { Write-Host "warning: $Message" -ForegroundColor Yellow }

function Show-Usage {
    Say @'
Install Canopy for the current user (no administrator rights).

  irm https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.ps1 | iex

With options:

  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.ps1))) [options]
  powershell -ExecutionPolicy Bypass -File install.ps1 [options]

Options:
  -Beta         Install the newest build, including pre-releases (the beta channel).
  -Version X    Install version X (for example 0.2.0) instead of the newest one.
  -NoPath       Do not add the install folder to the user PATH.
  -Uninstall    Remove Canopy: exactly what this script created. Settings and data stay.
  -Purge        With -Uninstall: also delete Canopy's settings, logs and caches (asks first).
  -Yes          Do not ask for confirmation (for -Purge).
  -Help         Show this help.

Environment:
  CANOPY_INSTALL_DIR   Install folder (default %LOCALAPPDATA%\Programs\Canopy).

"Apps & features" (or the installed uninstall.ps1) also uninstalls. Re-running the installer
upgrades in place; a failed upgrade keeps the installed version.
'@
}

# -----------------------------------------------------------------------------------------------
# Helpers

function Get-File([string]$Url, [string]$OutFile) {
    if ($Url -match '^https://') {
        # Windows PowerShell 5.1 may not offer TLS 1.2 by default.
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } elseif ($InsecureLocal -and $Url -match '^http://(127\.0\.0\.1|localhost):[0-9]+/') {
        # Test feed on this machine.
    } else {
        Fail "refusing to download from $Url"
    }
    $params = @{ Uri = $Url; OutFile = $OutFile; UseBasicParsing = $true; MaximumRedirection = 5 }
    if ($InsecureLocal -and $PSVersionTable.PSVersion.Major -ge 6) { $params['NoProxy'] = $true }
    Invoke-WebRequest @params
}

function Test-Version([string]$V) {
    return $V -match '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'
}

# Semantic version order: 1 when $A is newer than $B, -1 when older, 0 when equal.
function Compare-Version([string]$A, [string]$B) {
    $pa = $A -split '-', 2
    $pb = $B -split '-', 2
    $ca = $pa[0] -split '\.'
    $cb = $pb[0] -split '\.'
    for ($i = 0; $i -lt [Math]::Max($ca.Count, $cb.Count); $i++) {
        $x = 0; $y = 0
        if ($i -lt $ca.Count) { $x = [long]$ca[$i] }
        if ($i -lt $cb.Count) { $y = [long]$cb[$i] }
        if ($x -ne $y) { return [Math]::Sign($x - $y) }
    }
    $ra = if ($pa.Count -gt 1) { $pa[1] } else { '' }
    $rb = if ($pb.Count -gt 1) { $pb[1] } else { '' }
    if ($ra -eq $rb) { return 0 }
    if ($ra -eq '') { return 1 }
    if ($rb -eq '') { return -1 }
    $ia = $ra -split '\.'
    $ib = $rb -split '\.'
    for ($i = 0; $i -lt [Math]::Min($ia.Count, $ib.Count); $i++) {
        $na = $ia[$i] -match '^[0-9]+$'
        $nb = $ib[$i] -match '^[0-9]+$'
        if ($na -and $nb) {
            if ([long]$ia[$i] -ne [long]$ib[$i]) { return [Math]::Sign([long]$ia[$i] - [long]$ib[$i]) }
        } elseif ($na) { return -1 }
        elseif ($nb) { return 1 }
        else {
            $c = [string]::CompareOrdinal($ia[$i], $ib[$i])
            if ($c -ne 0) { return [Math]::Sign($c) }
        }
    }
    return [Math]::Sign($ia.Count - $ib.Count)
}

# The manifest of channel $Name, or $null when that channel has none (beta only).
function Get-Channel([string]$Name, [string]$Work) {
    $file = Join-Path $Work "$Name.json"
    try {
        Get-File "$FeedUrl/$Name.json" $file
    } catch [System.ApplicationException] {
        throw
    } catch {
        if ($Name -eq 'beta') { return $null }
        Fail "could not read the update manifest $FeedUrl/$Name.json: $($_.Exception.Message)"
    }
    $m = Get-Content -LiteralPath $file -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not (Test-Version ([string]$m.version))) { Fail "the update manifest $Name.json names no valid version" }
    return $m
}

function Get-Platform($Manifest, [string]$Key) {
    $platforms = $Manifest.PSObject.Properties['platforms']
    if ($null -eq $platforms -or $null -eq $platforms.Value) { return $null }
    $p = $platforms.Value.PSObject.Properties[$Key]
    if ($null -eq $p) { return $null }
    return $p.Value
}

function Confirm-Action([string]$Question) {
    if ($Yes) { return $true }
    try {
        $answer = Read-Host "$Question [y/N]"
    } catch {
        Fail "$($Question): cannot ask in this session; re-run with -Yes to confirm"
    }
    return $answer -match '^(y|yes)$'
}

function Test-CanopyRunning([string]$Root) {
    $exe = Join-Path $Root 'canopy.exe'
    $running = Get-Process -Name canopy -ErrorAction SilentlyContinue | Where-Object {
        try { $_.Path -eq $exe } catch { $false }
    }
    return [bool]$running
}

function Read-Receipt([string]$Root) {
    $r = @{}
    $file = Join-Path $Root '.canopy-install'
    if (Test-Path -LiteralPath $file) {
        foreach ($line in Get-Content -LiteralPath $file) {
            if ($line -match '^([a-z_]+)=(.*)$') { $r[$Matches[1]] = $Matches[2] }
        }
    }
    return $r
}

function Write-Receipt([string]$Root, [hashtable]$Values) {
    $lines = @("# Written by the Canopy installer: what it created outside $Root.")
    foreach ($k in ($Values.Keys | Sort-Object)) { $lines += "$k=$($Values[$k])" }
    Set-Content -LiteralPath (Join-Path $Root '.canopy-install') -Value $lines -Encoding UTF8
}

# The user PATH as stored (unexpanded), so entries such as %USERPROFILE%\bin survive.
function Get-UserPath {
    $key = Get-Item -LiteralPath 'HKCU:\Environment'
    return [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
}

function Set-UserPath([string]$Value) {
    if ($Value -eq '') {
        Remove-ItemProperty -LiteralPath 'HKCU:\Environment' -Name Path -ErrorAction SilentlyContinue
    } else {
        Set-ItemProperty -LiteralPath 'HKCU:\Environment' -Name Path -Value $Value -Type ExpandString
    }
    # Setting a user variable through .NET broadcasts WM_SETTINGCHANGE, so Explorer and new
    # terminals see the new PATH without signing out.
    [Environment]::SetEnvironmentVariable('CANOPY_INSTALLER_REFRESH', '1', 'User')
    [Environment]::SetEnvironmentVariable('CANOPY_INSTALLER_REFRESH', $null, 'User')
}

function Test-PathHas([string]$PathValue, [string]$Dir) {
    foreach ($p in ($PathValue -split ';')) {
        if ($p.TrimEnd('\') -ieq $Dir.TrimEnd('\')) { return $true }
    }
    return $false
}

function Remove-FromUserPath([string]$Dir) {
    $current = Get-UserPath
    if (-not (Test-PathHas $current $Dir)) { return }
    $kept = @($current -split ';' | Where-Object { $_ -ne '' -and $_.TrimEnd('\') -ine $Dir.TrimEnd('\') })
    Set-UserPath ($kept -join ';')
    Say "  removed $Dir from the user PATH"
}

function Get-StartMenuShortcut {
    return Join-Path ([Environment]::GetFolderPath('Programs')) 'Canopy.lnk'
}

function Get-ShortcutTarget([string]$Lnk) {
    if (-not (Test-Path -LiteralPath $Lnk)) { return $null }
    $shell = New-Object -ComObject WScript.Shell
    return $shell.CreateShortcut($Lnk).TargetPath
}

function Show-GitReport {
    # Windows PowerShell 5.1 turns a native command's stderr into errors, which 'Stop' would make
    # fatal: git prints to stderr when git-lfs is missing.
    $ErrorActionPreference = 'Continue'
    Say ''
    $git = Get-Command git -ErrorAction SilentlyContinue
    $lfs = $false
    if ($git) {
        try { Say ('git:      ' + (& git --version 2>$null)) } catch { Say 'git:      found' }
        try {
            $lfsVersion = & git lfs version 2>$null
            if ($LASTEXITCODE -eq 0 -and $lfsVersion) { $lfs = $true; Say "git-lfs:  $lfsVersion" }
        } catch { $lfs = $false }
    } else {
        Say 'git:      NOT FOUND. Canopy needs git 2.30 or newer on PATH.'
    }
    if (-not $lfs) { Say 'git-lfs:  not found (only needed for repositories that use Git LFS)' }
    if (-not $git -or -not $lfs) {
        Say '          Install Git for Windows (it includes Git LFS): winget install --id Git.Git -e'
        Say '          or download it from https://git-scm.com/download/win'
        Say '          (this installer never installs git for you).'
    }
}

# -----------------------------------------------------------------------------------------------
# Uninstall

function Invoke-ConfirmPurge {
    $existing = @(Get-PurgeTargets | Where-Object { Test-Path -LiteralPath $_ })
    if ($existing.Count -eq 0) {
        Say 'No Canopy settings or data found to purge.'
        return $false
    }
    Say "-Purge deletes Canopy's settings, logs and caches:"
    foreach ($d in $existing) { Say "  $d" }
    if (Confirm-Action 'Delete them?') { return $true }
    Say 'Keeping them.'
    return $false
}

function Invoke-Purge([bool]$Confirmed) {
    if ($Confirmed) {
        foreach ($d in Get-PurgeTargets) {
            if (Test-Path -LiteralPath $d) {
                Remove-Item -LiteralPath $d -Recurse -Force
                Say "  removed $d"
            }
        }
        foreach ($parent in @((Join-Path $env:APPDATA 'NecturaLabs'), (Join-Path $env:LOCALAPPDATA 'NecturaLabs'))) {
            if ((Test-Path -LiteralPath $parent) -and -not (Get-ChildItem -LiteralPath $parent -Force)) {
                Remove-Item -LiteralPath $parent -Force
            }
        }
    }
    Say ''
    Say "Tokens saved in Settings -> Accounts stay in Windows Credential Manager (service"
    Say "`"$AppId`"): remove them in Canopy before uninstalling, or in Credential Manager."
}

function Invoke-Uninstall([string]$Root) {
    $purgeOk = $false
    if ($Purge) { $purgeOk = Invoke-ConfirmPurge }
    if (-not (Test-Path -LiteralPath $Root)) {
        Say "Canopy is not installed in $Root; nothing to remove."
    } else {
        if (-not (Test-Path -LiteralPath (Join-Path $Root '.canopy-install'))) {
            Fail "$Root has no .canopy-install receipt, so this script did not create it; not touching it"
        }
        if (Test-CanopyRunning $Root) { Fail 'Canopy is running; close it and run the uninstaller again' }
        Say "Removing Canopy from $Root"
        $receipt = Read-Receipt $Root
        $exe = Join-Path $Root 'canopy.exe'
        $lnk = Get-StartMenuShortcut
        if ((Get-ShortcutTarget $lnk) -eq $exe) {
            Remove-Item -LiteralPath $lnk -Force
            Say "  removed $lnk"
        }
        if (Test-Path -LiteralPath $UninstallKey) {
            $props = Get-ItemProperty -LiteralPath $UninstallKey -ErrorAction SilentlyContinue
            $loc = $null
            if ($props -and $props.PSObject.Properties['InstallLocation']) { $loc = $props.InstallLocation }
            if ($loc -eq $Root) {
                Remove-Item -LiteralPath $UninstallKey -Recurse -Force
                Say '  removed the Apps & features entry'
            }
        }
        if ($receipt.ContainsKey('path') -and $receipt['path'] -eq '1') { Remove-FromUserPath $Root }
        Remove-Item -LiteralPath $Root -Recurse -Force
        Say "  removed $Root"
    }
    if ($Purge) {
        Invoke-Purge $purgeOk
    } else {
        Say 'Settings and data were kept (-Uninstall -Purge removes them).'
    }
}

# -----------------------------------------------------------------------------------------------
# Install

# Copies this script to $Dest (the uninstaller is the installer run as uninstall.ps1).
function Save-Self([string]$Dest) {
    $self = $ScriptFile
    if ($self -and (Test-Path -LiteralPath $self) -and ((Get-Content -LiteralPath $self -TotalCount 1) -match '^# canopy-installer')) {
        Copy-Item -LiteralPath $self -Destination $Dest -Force
        return
    }
    # Run through iex: fetch the published copy of this script again.
    try {
        Get-File "$FeedUrl/install.ps1" $Dest
        $first = Get-Content -LiteralPath $Dest -TotalCount 1
        $errors = $null
        [void][System.Management.Automation.Language.Parser]::ParseFile($Dest, [ref]$null, [ref]$errors)
        if ($first -notmatch '^# canopy-installer' -or $errors.Count -gt 0) { throw 'not the installer' }
    } catch {
        Warn 'could not save uninstall.ps1; uninstall from Apps & features or with the installer and -Uninstall'
        Remove-Item -LiteralPath $Dest -Force -ErrorAction SilentlyContinue
    }
}

function Move-Entries([string]$From, [string]$To) {
    foreach ($e in @(Get-ChildItem -LiteralPath $From -Force | Where-Object { -not $_.Name.StartsWith('.') })) {
        Move-Item -LiteralPath $e.FullName -Destination (Join-Path $To $e.Name)
    }
}

function Invoke-Install([string]$Root) {
    if (-not [Environment]::Is64BitOperatingSystem) { Fail 'Canopy needs 64-bit Windows' }
    $arch = $env:PROCESSOR_ARCHITEW6432
    if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
    $key = 'windows-x86_64'
    $target = 'x86_64-pc-windows-msvc'
    if ($arch -eq 'ARM64') { Say 'Windows on Arm: installing the x64 build, which Windows runs through emulation.' }
    Say "Installing Canopy for Windows ($key)"

    if ((Test-Path -LiteralPath $Root) -and -not (Test-Path -LiteralPath $Root -PathType Container)) {
        Fail "$Root exists and is not a folder"
    }
    $receiptFile = Join-Path $Root '.canopy-install'
    if ((Test-Path -LiteralPath $Root) -and -not (Test-Path -LiteralPath $receiptFile) -and (Get-ChildItem -LiteralPath $Root -Force)) {
        Fail "$Root is not empty and was not created by this installer; choose another CANOPY_INSTALL_DIR"
    }
    $createdRoot = $false
    if (-not (Test-Path -LiteralPath $Root)) {
        New-Item -ItemType Directory -Path $Root -Force | Out-Null
        $createdRoot = $true
    }
    $lock = Join-Path $Root '.lock'
    try {
        New-Item -ItemType Directory -Path $lock -ErrorAction Stop | Out-Null
    } catch {
        Fail "another Canopy install is running (remove $lock if it is not)"
    }
    $work = Join-Path $Root ('.staging.' + [IO.Path]::GetRandomFileName())
    $succeeded = $false
    try {
        if (-not (Test-Path -LiteralPath $receiptFile)) { Write-Receipt $Root @{} }
        # Leftovers of an upgrade whose old canopy.exe was still in use.
        Get-ChildItem -LiteralPath $Root -Force -Filter '.previous.*' | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Path $work | Out-Null
        $oldVersion = ''
        $versionFile = Join-Path $Root 'VERSION'
        if (Test-Path -LiteralPath $versionFile) { $oldVersion = (Get-Content -LiteralPath $versionFile -TotalCount 1).Trim() }

        # Which release.
        $pin = $Version.TrimStart('v')
        if ($pin -and -not (Test-Version $pin)) { Fail "-Version $pin is not a version like 0.2.0 or 0.3.0-beta.1" }
        Say 'Reading the update manifest'
        $use = Get-Channel 'latest' $work
        if ($Beta -or $pin) {
            $betaManifest = Get-Channel 'beta' $work
            if ($betaManifest) {
                if ($pin) {
                    if ($betaManifest.version -eq $pin) { $use = $betaManifest }
                } elseif ((Compare-Version $betaManifest.version $use.version) -gt 0) {
                    $use = $betaManifest
                }
            }
        }
        if (-not $pin -or $pin -eq $use.version) {
            $ver = [string]$use.version
            $p = Get-Platform $use $key
            if ($null -eq $p) {
                $keys = @()
                if ($use.PSObject.Properties['platforms'] -and $use.platforms) { $keys = @($use.platforms.PSObject.Properties | ForEach-Object { $_.Name }) }
                $list = if ($keys.Count) { $keys -join ', ' } else { 'none' }
                Fail "Canopy $ver has no build for Windows ($key) yet. Builds in this release: $list"
            }
            $url = [string]$p.url
            $sha = [string]$p.sha256
            $asset = $url.Substring($url.LastIndexOf('/') + 1)
        } else {
            $ver = $pin
            $asset = "canopy-$ver-$target.zip"
            Say "Reading the checksums of Canopy $ver"
            $sums = Join-Path $work 'SHA256SUMS'
            try { Get-File "$ReleasesUrl/canopy-v$ver/SHA256SUMS" $sums } catch [System.ApplicationException] { throw } catch {
                Fail "Canopy $ver is not published (no $ReleasesUrl/canopy-v$ver/SHA256SUMS)"
            }
            $sha = ''
            foreach ($line in Get-Content -LiteralPath $sums) {
                $parts = $line -split '\s+', 2
                if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $asset) { $sha = $parts[0]; break }
            }
            if (-not $sha) { Fail "Canopy $ver has no build for Windows ($key)" }
            $url = "$ReleasesUrl/canopy-v$ver/$asset"
        }
        # Only ever download Canopy's own archive from Canopy's own release in the feed repository.
        if ($url -cne "$ReleasesUrl/canopy-v$ver/$asset") { Fail "the manifest points outside Canopy's releases: $url" }
        if ($asset -cne "canopy-$ver-$target.zip") { Fail "unexpected archive name in the manifest: $asset" }
        if ($sha -cnotmatch '^[0-9a-f]{64}$') { Fail "the manifest has no valid sha256 for $asset" }

        # Download and verify.
        Say "Downloading $url"
        $zip = Join-Path $work $asset
        try { Get-File $url $zip } catch [System.ApplicationException] { throw } catch { Fail "download failed: $url ($($_.Exception.Message))" }
        $got = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($got -ne $sha) {
            Fail "checksum mismatch for $asset`n  expected $sha`n  got      $got`nNothing was installed. Try again later; if it persists, report it."
        }
        Say "Checksum verified (sha256 $got)"
        $x = Join-Path $work 'x'
        Expand-Archive -LiteralPath $zip -DestinationPath $x
        $src = Join-Path $x "canopy-$ver-$target"
        if (-not (Test-Path -LiteralPath (Join-Path $src 'canopy.exe'))) { Fail "$asset has no canopy-$ver-$target\canopy.exe" }

        # Stage the new folder content.
        $new = Join-Path $work 'new'
        New-Item -ItemType Directory -Path $new | Out-Null
        foreach ($name in @('canopy.exe', 'README.md', 'LICENSE', 'legal')) {
            $from = Join-Path $src $name
            if (Test-Path -LiteralPath $from) { Copy-Item -LiteralPath $from -Destination (Join-Path $new $name) -Recurse }
        }
        Set-Content -LiteralPath (Join-Path $new 'VERSION') -Value $ver -Encoding ASCII
        Save-Self (Join-Path $new 'uninstall.ps1')

        # Swap: move the old content aside, the new content in; put the old back on failure.
        if (Test-CanopyRunning $Root) { Fail 'Canopy is running; close it and run the installer again' }
        $previous = Join-Path $Root ('.previous.' + [IO.Path]::GetRandomFileName())
        New-Item -ItemType Directory -Path $previous | Out-Null
        try {
            Move-Entries $Root $previous
            Move-Entries $new $Root
        } catch {
            Get-ChildItem -LiteralPath $Root -Force | Where-Object { -not $_.Name.StartsWith('.') } | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
            Move-Entries $previous $Root
            Fail "could not move the new version into $Root; the previous one was kept ($($_.Exception.Message))"
        }
        Remove-Item -LiteralPath $previous -Recurse -Force -ErrorAction SilentlyContinue
        $succeeded = $true

        # Register: Start Menu shortcut, Apps & features entry, user PATH.
        $receipt = Read-Receipt $Root
        $receipt['version'] = $ver
        $exe = Join-Path $Root 'canopy.exe'
        $lnk = Get-StartMenuShortcut
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($lnk)
        $shortcut.TargetPath = $exe
        $shortcut.WorkingDirectory = $env:USERPROFILE
        $shortcut.IconLocation = "$exe,0"
        $shortcut.Description = 'Canopy, a fast Git client'
        $shortcut.Save()
        $receipt['shortcut'] = $lnk

        $uninstallScript = Join-Path $Root 'uninstall.ps1'
        if (-not (Test-Path -LiteralPath $UninstallKey)) { New-Item -Path $UninstallKey -Force | Out-Null }
        $size = [int]((Get-ChildItem -LiteralPath $Root -Recurse -Force -File | Measure-Object -Property Length -Sum).Sum / 1KB)
        $entries = @{
            DisplayName     = 'Canopy'
            DisplayVersion  = $ver
            Publisher       = 'NecturaLabs'
            InstallLocation = $Root
            DisplayIcon     = "$exe,0"
            InstallDate     = (Get-Date -Format 'yyyyMMdd')
            UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$uninstallScript`""
            QuietUninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$uninstallScript`" -Yes"
            URLInfoAbout    = 'https://github.com/NecturaLabs/app-updates/releases'
        }
        foreach ($k in $entries.Keys) { Set-ItemProperty -LiteralPath $UninstallKey -Name $k -Value $entries[$k] -Type String }
        Set-ItemProperty -LiteralPath $UninstallKey -Name EstimatedSize -Value $size -Type DWord
        Set-ItemProperty -LiteralPath $UninstallKey -Name NoModify -Value 1 -Type DWord
        Set-ItemProperty -LiteralPath $UninstallKey -Name NoRepair -Value 1 -Type DWord
        $receipt['uninstall_key'] = $UninstallKey

        $pathNote = ''
        if ($NoPath) {
            if ($receipt.ContainsKey('path') -and $receipt['path'] -eq '1') { Remove-FromUserPath $Root }
            $receipt['path'] = '0'
        } else {
            $userPath = Get-UserPath
            if (-not (Test-PathHas $userPath $Root)) {
                $sep = if ($userPath -eq '' -or $userPath.EndsWith(';')) { '' } else { ';' }
                Set-UserPath "$userPath$sep$Root"
                $receipt['path'] = '1'
                $pathNote = 'Open a new terminal to use the canopy command.'
            } elseif (-not $receipt.ContainsKey('path')) {
                $receipt['path'] = '0' # it was already there; not ours to remove
            }
            if (-not (Test-PathHas $env:Path $Root)) { $env:Path = "$env:Path;$Root" }
        }
        Write-Receipt $Root $receipt

        Say ''
        if (-not $oldVersion) { Say "Installed Canopy $ver in $Root" }
        elseif ($oldVersion -eq $ver) { Say "Reinstalled Canopy $ver in $Root" }
        else { Say "Updated Canopy $oldVersion -> $ver in $Root" }
        Say "Start Menu: $lnk"
        if (-not $NoPath) { Say "Command:    canopy  (the folder is on your user PATH)" }
        Say 'Remove:     Settings -> Apps -> Installed apps -> Canopy, or uninstall.ps1 in the folder'
        if ($pathNote) { Say $pathNote }
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $lock -Force -ErrorAction SilentlyContinue
        if ($createdRoot -and -not $succeeded) {
            Remove-Item -LiteralPath $Root -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# -----------------------------------------------------------------------------------------------

function Invoke-Main {
    if ($Help) { Show-Usage; return }
    if ($ExtraArgs.Count -gt 0) { Fail "unknown option: $($ExtraArgs -join ' ') (see -Help)" }
    if ($env:OS -ne 'Windows_NT') {
        Fail 'install.ps1 is for Windows. On Linux and macOS run: curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh'
    }
    if ($Purge -and -not $Uninstall) { Fail '-Purge only goes with -Uninstall' }

    if ($env:CANOPY_INSTALL_BASE_URL) {
        # Test-only: point the script at a fake feed on this machine (plain-HTTP loopback only).
        if ($env:CANOPY_INSTALL_BASE_URL -notmatch '^http://(127\.0\.0\.1|localhost):[0-9]+/?$') {
            Fail 'CANOPY_INSTALL_BASE_URL is for tests and only accepts http://127.0.0.1:<port> or http://localhost:<port>'
        }
        $base = $env:CANOPY_INSTALL_BASE_URL.TrimEnd('/')
        $script:FeedUrl = "$base/canopy"
        $script:ReleasesUrl = "$base/releases/download"
        $script:InsecureLocal = $true
        Warn "test mode: using the feed at $base"
    }

    $mode = if ($Uninstall) { 'uninstall' } else { 'install' }
    $root = Join-Path $env:LOCALAPPDATA 'Programs\Canopy'
    if ($env:CANOPY_INSTALL_DIR) { $root = $env:CANOPY_INSTALL_DIR }
    if ($ScriptFile -and (Split-Path -Leaf $ScriptFile) -eq 'uninstall.ps1') {
        # The installed uninstall.ps1 always removes the folder it sits in.
        $mode = 'uninstall'
        $root = Split-Path -Parent $ScriptFile
    }
    $root = [IO.Path]::GetFullPath($root).TrimEnd('\')
    $forbidden = @($env:LOCALAPPDATA, (Join-Path $env:LOCALAPPDATA 'Programs'), $env:APPDATA, $env:USERPROFILE,
        $env:ProgramFiles, ${env:ProgramFiles(x86)}, $env:SystemRoot, [IO.Path]::GetPathRoot($root)) |
        Where-Object { $_ } | ForEach-Object { $_.TrimEnd('\') }
    if ($forbidden -contains $root) { Fail "refusing to use $root as Canopy's own folder; it must be a folder for Canopy alone" }

    if ($mode -eq 'uninstall') {
        Invoke-Uninstall $root
    } else {
        Invoke-Install $root
        Show-GitReport
    }
}

try {
    Invoke-Main
    if ($ScriptFile) { exit 0 }
} catch [System.ApplicationException] {
    Write-Host "error: $($_.Exception.Message)" -ForegroundColor Red
    # Through `irm | iex`, `exit` would close the user's PowerShell window: throw instead.
    if ($ScriptFile) { exit 1 }
    throw 'The Canopy installer failed (see the error above).'
} catch {
    Write-Host "error: $($_.Exception.Message)" -ForegroundColor Red
    if ($ScriptFile) { exit 1 }
    throw
}
