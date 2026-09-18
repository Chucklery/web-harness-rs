param(
    [Parameter(Mandatory = $true)]
    [string]$Target,
    [Parameter(Mandatory = $true)]
    [string]$Version
)

$ErrorActionPreference = "Stop"
$Binary = Join-Path "target" "$Target/release/web-harness.exe"
$ArchiveName = "web-harness-$Version-$Target.zip"
$DistDir = Join-Path (Get-Location) "dist"
$ArchivePath = Join-Path $DistDir $ArchiveName
$Stage = Join-Path ([System.IO.Path]::GetTempPath()) ("web-harness-release-" + [guid]::NewGuid())

if (-not (Test-Path $Binary -PathType Leaf)) {
    throw "release binary not found: $Binary"
}

New-Item -ItemType Directory -Path $Stage -Force | Out-Null
New-Item -ItemType Directory -Path $DistDir -Force | Out-Null
try {
    Copy-Item $Binary (Join-Path $Stage "web-harness.exe")
    $TunnelDir = Join-Path $Stage "libexec/web-harness"
    New-Item -ItemType Directory -Path $TunnelDir -Force | Out-Null
    & "$PSScriptRoot/fetch-tunnel-client.ps1" -Target $Target -Destination $TunnelDir | Out-Null

    Copy-Item "LICENSE" $Stage
    Copy-Item "NOTICE" $Stage
    Copy-Item "README.md" $Stage
    Copy-Item "THIRD_PARTY_NOTICES.md" $Stage

    if (Test-Path $ArchivePath) {
        Remove-Item $ArchivePath -Force
    }
    Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $ArchivePath -CompressionLevel Optimal

    $Hash = (Get-FileHash -Algorithm SHA256 $ArchivePath).Hash.ToLowerInvariant()
    Set-Content -Path "$ArchivePath.sha256" -Value "$Hash  $ArchiveName"
    Write-Output $ArchivePath
}
finally {
    Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
}
