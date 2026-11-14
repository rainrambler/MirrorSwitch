param(
    [string]$Configuration = "release"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $cargoToml = Get-Content (Join-Path $root "Cargo.toml") -Raw
    if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
        throw "unable to read package version from Cargo.toml"
    }
    $version = $Matches[1]

    cargo build --$Configuration
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }

    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force -Path $dist | Out-Null

    $exe = Join-Path $root "target\$Configuration\MirrorSwitch.exe"
    if (-not (Test-Path $exe)) { throw "missing build output: $exe" }

    Copy-Item $exe (Join-Path $dist "MirrorSwitch.exe") -Force
    Copy-Item (Join-Path $root "README.md") (Join-Path $dist "README.md") -Force
    Copy-Item (Join-Path $root "LICENSE") (Join-Path $dist "LICENSE") -Force
    $distDocs = Join-Path $dist "docs"
    if (Test-Path $distDocs) { Remove-Item $distDocs -Recurse -Force }
    Copy-Item (Join-Path $root "docs") $distDocs -Recurse -Force

    $zipName = "MirrorSwitch-v$version-win64.zip"
    $zip = Join-Path $dist $zipName
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path @(
        (Join-Path $dist "MirrorSwitch.exe"),
        (Join-Path $dist "README.md"),
        (Join-Path $dist "LICENSE"),
        (Join-Path $dist "docs")
    ) -DestinationPath $zip -CompressionLevel Optimal

    $hash = Get-FileHash -Algorithm SHA256 -Path $zip
    "$($hash.Hash.ToLower())  $zipName" |
        Set-Content -Path (Join-Path $dist "SHA256SUMS.txt") -Encoding ascii

    Write-Host "Built: $zip"
    Write-Host "SHA256: $($hash.Hash.ToLower())"
}
finally {
    Pop-Location
}
