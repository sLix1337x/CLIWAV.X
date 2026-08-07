#Requires -Version 5.1
$ErrorActionPreference = "Stop"

$repoOwner = "sLix1337x"
$repoName  = "CLIWAV.X"
$assetName = "cliwavx.exe"

$installDir = Join-Path $env:LOCALAPPDATA "CLIWAV.X"
$binaryPath = Join-Path $installDir $assetName

Write-Host "Installing CLIWAV.X..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

$downloadUrl = "https://github.com/$repoOwner/$repoName/releases/download/latest/$assetName"
$tempFile = Join-Path $env:TEMP $assetName

Write-Host "Downloading $assetName from GitHub Releases..."
try {
    Invoke-WebRequest -Uri $downloadUrl -OutFile $tempFile -UseBasicParsing
} catch {
    Write-Host "Failed to download the release binary." -ForegroundColor Red
    Write-Host "Make sure you are connected to the internet and the latest release exists at:"
    Write-Host "https://github.com/$repoOwner/$repoName/releases/latest" -ForegroundColor Yellow
    throw
}

Copy-Item -Path $tempFile -Destination $binaryPath -Force
Remove-Item -Path $tempFile -Force -ErrorAction SilentlyContinue

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    Write-Host "Added $installDir to your user PATH." -ForegroundColor Green
}

Write-Host "CLIWAV.X installed to: $binaryPath" -ForegroundColor Green
Write-Host "Restart your terminal and run 'cliwavx' from anywhere." -ForegroundColor Green
