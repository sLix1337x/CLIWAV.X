#Requires -Version 5.1
$ErrorActionPreference = "Stop"

$repoOwner = "sLix1337x"
$repoName  = "CLIWAV.X"
# Two identical binaries under different names, so both "cliwavx" and the
# shorter "wavx" work as commands.
$assetNames = @("cliwavx.exe", "wavx.exe")

$installDir = Join-Path $env:LOCALAPPDATA "CLIWAV.X"

function Test-Command($cmd) {
    return [bool](Get-Command $cmd -ErrorAction SilentlyContinue)
}

function Test-Winget() {
    return Test-Command winget
}

function Install-WingetPackage($packageId, $packageName) {
    Write-Host "Installing $packageName via winget..." -ForegroundColor Cyan
    winget install --id=$packageId -e --silent --accept-package-agreements --accept-source-agreements
    if ($LASTEXITCODE -ne 0) {
        throw "winget install of $packageName failed (exit $LASTEXITCODE). Try running this script as Administrator."
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

Write-Host "Installing CLIWAV.X and its dependencies..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

# --- mpv ---
if (-not (Test-Command mpv)) {
    if (Test-Winget) {
        try {
            Install-WingetPackage -packageId "shinchiro.mpv" -packageName "mpv"
        } catch {
            Write-Warning "Could not install mpv automatically: $_"
            Write-Host "Please install mpv manually from https://mpv.io/ and make sure it is on your PATH." -ForegroundColor Yellow
        }
    } else {
        Write-Warning "winget was not found. Cannot install mpv automatically."
        Write-Host "Please install mpv manually from https://mpv.io/ and make sure it is on your PATH." -ForegroundColor Yellow
    }
} else {
    Write-Host "mpv is already installed." -ForegroundColor Green
}

# --- yt-dlp ---
if (-not (Test-Command yt-dlp)) {
    if (Test-Winget) {
        try {
            Install-WingetPackage -packageId "yt-dlp.yt-dlp" -packageName "yt-dlp"
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

# --- CLIWAV.X binaries ---
foreach ($assetName in $assetNames) {
    $downloadUrl = "https://github.com/$repoOwner/$repoName/releases/download/latest/$assetName"
    $tempFile = Join-Path $env:TEMP $assetName

    Write-Host "Downloading $assetName from GitHub Releases..." -ForegroundColor Cyan
    try {
        Invoke-WebRequest -Uri $downloadUrl -OutFile $tempFile -UseBasicParsing
    } catch {
        Write-Host "Failed to download $assetName." -ForegroundColor Red
        Write-Host "Make sure you are connected to the internet and the latest release exists at:"
        Write-Host "https://github.com/$repoOwner/$repoName/releases/latest" -ForegroundColor Yellow
        throw
    }

    Copy-Item -Path $tempFile -Destination (Join-Path $installDir $assetName) -Force
    Remove-Item -Path $tempFile -Force -ErrorAction SilentlyContinue
}

# --- PATH ---
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    Write-Host "Added $installDir to your user PATH." -ForegroundColor Green
}

Write-Host "CLIWAV.X installed to: $installDir" -ForegroundColor Green
Write-Host "Restart your terminal and run 'cliwavx' (or the shorter 'wavx') from anywhere." -ForegroundColor Green

if (-not (Test-Command mpv) -or -not (Test-Command yt-dlp)) {
    Write-Host "`nNOTE: mpv or yt-dlp could not be verified on PATH. They may need a terminal restart to be detected." -ForegroundColor Yellow
    Write-Host "If they are still missing after restarting, install them manually or re-run this script as Administrator." -ForegroundColor Yellow
}
