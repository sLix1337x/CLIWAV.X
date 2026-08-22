#Requires -Version 5.1
$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repoOwner = "sLix1337x"
$repoName  = "CLIWAV.X"
$assetName = "cliwavx.exe"

$installDir = Join-Path $env:LOCALAPPDATA "CLIWAV.X"

function Test-Command($cmd) {
    return [bool](Get-Command $cmd -ErrorAction SilentlyContinue)
}

# Pull the machine + user PATH back out of the registry into this session, so a
# dependency installed a few lines ago is actually findable by Test-Command
# instead of only after the user restarts their terminal.
function Update-SessionPath() {
    $machine = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $user    = [Environment]::GetEnvironmentVariable("Path", "User")
    $env:Path = ($machine, $user | Where-Object { $_ }) -join ';'
}

function Add-ToUserPath($dir) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @($userPath -split ';' | Where-Object { $_ })
    if ($entries -notcontains $dir) {
        [Environment]::SetEnvironmentVariable("Path", (($entries + $dir) -join ';'), "User")
        Write-Host "Added $dir to your user PATH." -ForegroundColor Green
    }
    if (($env:Path -split ';') -notcontains $dir) { $env:Path = "$env:Path;$dir" }
}

# "wavx" is a hard link to cliwavx.exe, not a second download: same file on
# disk under a second name, so `wavx` works as a shorter command for free.
# Falls back to a plain copy if hard links aren't available for some reason.
function New-WavxAlias($target, $linkPath) {
    if (Test-Path $linkPath) { Remove-Item $linkPath -Force }
    try {
        New-Item -ItemType HardLink -Path $linkPath -Value $target -ErrorAction Stop | Out-Null
    } catch {
        Copy-Item -Path $target -Destination $linkPath -Force
    }
}

function Test-Winget() {
    return Test-Command winget
}

function Install-WingetPackage($packageId, $packageName) {
    Write-Host "Installing $packageName via winget..." -ForegroundColor Cyan
    # --source winget is not optional: if the msstore source is unreachable
    # (corporate DNS, a hosts-file blocklist, a debloat script), winget reports
    # the search as ambiguous and refuses to install anything at all. Pinning
    # the source skips msstore entirely. --disable-interactivity keeps it from
    # blocking on a prompt when this runs through `irm ... | iex`.
    $wingetArgs = @(
        "install", "--id", $packageId, "-e",
        "--source", "winget", "--silent",
        "--accept-package-agreements", "--accept-source-agreements",
        "--disable-interactivity"
    )
    & winget @wingetArgs
    if ($LASTEXITCODE -ne 0) {
        throw "winget install of $packageName failed (exit $LASTEXITCODE)."
    }
}

function Install-YtDlpPortable() {
    $ytDlpPath = Join-Path $installDir "yt-dlp.exe"
    if (Test-Path $ytDlpPath) { return }

    Write-Host "Downloading portable yt-dlp.exe to $installDir..." -ForegroundColor Cyan
    $url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
    Invoke-WebRequest -Uri $url -OutFile $ytDlpPath -UseBasicParsing
    Write-Host "yt-dlp saved to $ytDlpPath" -ForegroundColor Green
}

# Same idea as the yt-dlp fallback, for mpv. The shinchiro builds ship as .7z,
# which Windows' built-in tar.exe (libarchive) can extract without 7-Zip.
function Install-MpvPortable() {
    $mpvDir = Join-Path $installDir "mpv"
    if (Test-Path (Join-Path $mpvDir "mpv.exe")) { return $mpvDir }

    Write-Host "Downloading portable mpv to $mpvDir..." -ForegroundColor Cyan
    if (-not (Test-Command tar)) { throw "tar.exe is required to unpack mpv and was not found." }

    $api = "https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest"
    $release = Invoke-RestMethod -Uri $api -UseBasicParsing -Headers @{ "User-Agent" = "CLIWAV.X-installer" }
    $asset = $release.assets | Where-Object { $_.name -match '^mpv-x86_64-\d' } | Select-Object -First 1
    if (-not $asset) { throw "Could not find an mpv build in the latest shinchiro release." }

    $archive = Join-Path $env:TEMP $asset.name
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archive -UseBasicParsing
    New-Item -ItemType Directory -Force -Path $mpvDir | Out-Null
    & tar.exe -xf $archive -C $mpvDir
    $tarExit = $LASTEXITCODE
    Remove-Item $archive -Force -ErrorAction SilentlyContinue
    if ($tarExit -ne 0 -or -not (Test-Path (Join-Path $mpvDir "mpv.exe"))) {
        throw "Failed to unpack the mpv archive (tar exit $tarExit)."
    }

    Write-Host "mpv saved to $mpvDir" -ForegroundColor Green
    return $mpvDir
}

# winget's mpv package installs into Program Files but registers nothing on
# PATH, so a "successful" install still leaves mpv unusable. Go find it.
function Find-InstalledMpvDir() {
    $candidates = @(
        (Join-Path $env:ProgramFiles "MPV Player"),
        (Join-Path $env:ProgramFiles "mpv"),
        (Join-Path ${env:ProgramFiles(x86)} "MPV Player"),
        (Join-Path ${env:ProgramFiles(x86)} "mpv"),
        (Join-Path $env:LOCALAPPDATA "Programs\mpv")
    )
    foreach ($dir in $candidates) {
        if ($dir -and (Test-Path (Join-Path $dir "mpv.exe"))) { return $dir }
    }
    return $null
}

Write-Host "Installing CLIWAV.X and its dependencies..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Update-SessionPath

# --- mpv ---
if (-not (Test-Command mpv)) {
    $mpvInstalled = $false
    if (Test-Winget) {
        try {
            Install-WingetPackage -packageId "shinchiro.mpv" -packageName "mpv"
            Update-SessionPath
            $mpvDir = Find-InstalledMpvDir
            if ($mpvDir) { Add-ToUserPath $mpvDir }
            $mpvInstalled = Test-Command mpv
        } catch {
            Write-Warning "winget install of mpv failed, falling back to portable download: $_"
        }
    } else {
        Write-Warning "winget was not found. Downloading portable mpv instead."
    }

    if (-not $mpvInstalled) {
        try {
            $portableMpvDir = Install-MpvPortable
            Add-ToUserPath $portableMpvDir
        } catch {
            Write-Warning "Could not install mpv automatically: $_"
            Write-Host "Please install mpv manually from https://mpv.io/ and make sure it is on your PATH." -ForegroundColor Yellow
        }
    }
} else {
    Write-Host "mpv is already installed." -ForegroundColor Green
}

# --- yt-dlp ---
if (-not (Test-Command yt-dlp)) {
    if (Test-Winget) {
        try {
            Install-WingetPackage -packageId "yt-dlp.yt-dlp" -packageName "yt-dlp"
            Update-SessionPath
        } catch {
            Write-Warning "winget install of yt-dlp failed, falling back to portable download: $_"
            Install-YtDlpPortable
        }
    } else {
        Write-Warning "winget was not found. Downloading portable yt-dlp.exe instead."
        Install-YtDlpPortable
    }
} else {
    Write-Host "yt-dlp is already installed." -ForegroundColor Green
}

# --- CLIWAV.X binary ---
$downloadUrl = "https://github.com/$repoOwner/$repoName/releases/download/latest/$assetName"
$tempFile = Join-Path $env:TEMP $assetName

Write-Host "Downloading $assetName from GitHub Releases..." -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $downloadUrl -OutFile $tempFile -UseBasicParsing
} catch {
    Write-Host "Failed to download the release binary." -ForegroundColor Red
    Write-Host "Make sure you are connected to the internet and the latest release exists at:"
    Write-Host "https://github.com/$repoOwner/$repoName/releases/latest" -ForegroundColor Yellow
    throw
}

$exe = Join-Path $installDir $assetName
Copy-Item -Path $tempFile -Destination $exe -Force
Remove-Item -Path $tempFile -Force -ErrorAction SilentlyContinue
New-WavxAlias $exe (Join-Path $installDir "wavx.exe")

# --- PATH ---
Add-ToUserPath $installDir

Write-Host "CLIWAV.X installed to: $installDir" -ForegroundColor Green
Write-Host "Run 'cliwavx' (or the shorter 'wavx') from anywhere." -ForegroundColor Green

$missing = @()
if (-not (Test-Command mpv))    { $missing += "mpv" }
if (-not (Test-Command yt-dlp)) { $missing += "yt-dlp" }
if ($missing.Count -gt 0) {
    Write-Host "`nNOTE: $($missing -join ' and ') could not be verified on PATH." -ForegroundColor Yellow
    Write-Host "Restart your terminal; if they are still missing, install them manually." -ForegroundColor Yellow
}
