$ErrorActionPreference = "Stop"

$installDir = Join-Path $env:LOCALAPPDATA "CLIWAV.X"
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

Write-Host "Building CLIWAV.X release binary..."
cargo build --release
if ($LASTEXITCODE -ne 0) {
    throw "cargo build --release failed"
}

$src = Join-Path $PSScriptRoot "target\release\cliwavx.exe"
$dst = Join-Path $installDir "cliwavx.exe"
Copy-Item $src $dst -Force

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    Write-Host "Added $installDir to your user PATH."
}

Write-Host "Installation complete. You can now run 'cliwavx' from any terminal."
