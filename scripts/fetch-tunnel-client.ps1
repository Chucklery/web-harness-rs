param(
    [Parameter(Mandatory = $true)]
    [string]$Target,
    [Parameter(Mandatory = $true)]
    [string]$Destination
)

$ErrorActionPreference = "Stop"
$Version = "0.0.14"
$ChecksumsSha256 = "3c09650be77841e72747951f415b7be32b01be46ca7f534d873fe162c1fb6887"
$PrimaryBaseUrl = "https://persistent.oaistatic.com/tunnel-client/v$Version"
$FallbackBaseUrl = "https://github.com/openai/tunnel-client/releases/download/v$Version"

switch ($Target) {
    "x86_64-pc-windows-msvc" {
        $Platform = "windows-amd64"
    }
    "aarch64-pc-windows-msvc" {
        $Platform = "windows-arm64"
    }
    default {
        throw "unsupported tunnel-client Windows target: $Target"
    }
}

$Stem = "tunnel-client-runtime-v$Version-$Platform"
$Archive = "$Stem.zip"
$PrimaryUrl = "$PrimaryBaseUrl/$Archive"
$FallbackUrl = "$FallbackBaseUrl/$Archive"
$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("web-harness-tunnel-" + [guid]::NewGuid())
$ArchivePath = Join-Path $TempDir $Archive
$ChecksumsPath = Join-Path $TempDir "SHA256SUMS.txt"
$ExtractDir = Join-Path $TempDir "extracted"

New-Item -ItemType Directory -Path $TempDir -Force | Out-Null
try {
    try {
        Invoke-WebRequest -Uri "$PrimaryBaseUrl/SHA256SUMS.txt" -OutFile $ChecksumsPath -UseBasicParsing
    }
    catch {
        Write-Warning "primary OpenAI CDN download failed for SHA256SUMS.txt; trying GitHub release fallback"
        Invoke-WebRequest -Uri "$FallbackBaseUrl/SHA256SUMS.txt" -OutFile $ChecksumsPath -UseBasicParsing
    }

    $ActualChecksumsSha256 = (Get-FileHash -Algorithm SHA256 $ChecksumsPath).Hash.ToLowerInvariant()
    if ($ActualChecksumsSha256 -ne $ChecksumsSha256) {
        throw "tunnel-client checksum manifest mismatch"
    }

    $ChecksumLine = Get-Content $ChecksumsPath | Where-Object { $_ -match ("^[0-9a-fA-F]{64}\\s+" + [regex]::Escape($Archive) + "$") } | Select-Object -First 1
    if (-not $ChecksumLine) {
        throw "missing checksum for $Archive in upstream SHA256SUMS.txt"
    }
    $ExpectedSha256 = ($ChecksumLine -split "\\s+")[0].ToLowerInvariant()

    try {
        Invoke-WebRequest -Uri $PrimaryUrl -OutFile $ArchivePath -UseBasicParsing
    }
    catch {
        Write-Warning "primary OpenAI CDN download failed; trying GitHub release fallback"
        Invoke-WebRequest -Uri $FallbackUrl -OutFile $ArchivePath -UseBasicParsing
    }

    $ActualSha256 = (Get-FileHash -Algorithm SHA256 $ArchivePath).Hash.ToLowerInvariant()
    if ($ActualSha256 -ne $ExpectedSha256) {
        throw "tunnel-client runtime checksum mismatch for $Archive; expected $ExpectedSha256, actual $ActualSha256"
    }

    New-Item -ItemType Directory -Path $ExtractDir -Force | Out-Null
    Expand-Archive -Path $ArchivePath -DestinationPath $ExtractDir -Force
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    $SourceBinary = Join-Path $ExtractDir "tunnel-client-runtime.exe"
    $OutputBinary = Join-Path $Destination "tunnel-client.exe"
    if (-not (Test-Path $SourceBinary -PathType Leaf)) {
        throw "runtime binary missing from $Archive"
    }
    Copy-Item $SourceBinary $OutputBinary -Force

    foreach ($Evidence in @("LICENSE", "NOTICE", "$Stem-licenses.txt", "$Stem.spdx.json")) {
        $SourceEvidence = Join-Path $ExtractDir $Evidence
        if (-not (Test-Path $SourceEvidence -PathType Leaf)) {
            throw "required runtime evidence missing from $Archive"
        }
        Copy-Item $SourceEvidence (Join-Path $Destination $Evidence) -Force
    }

    Set-Content -Path (Join-Path $Destination "UPSTREAM_VERSION") -Value "v$Version" -NoNewline
    Set-Content -Path (Join-Path $Destination "UPSTREAM_FLAVOR") -Value "runtime" -NoNewline
    Set-Content -Path (Join-Path $Destination "UPSTREAM_ARTIFACT") -Value $Archive -NoNewline
    Set-Content -Path (Join-Path $Destination "UPSTREAM_URL") -Value $PrimaryUrl -NoNewline
    Set-Content -Path (Join-Path $Destination "UPSTREAM_FALLBACK_URL") -Value $FallbackUrl -NoNewline
    Set-Content -Path (Join-Path $Destination "UPSTREAM_SHA256") -Value $ExpectedSha256 -NoNewline

    Write-Output $OutputBinary
}
finally {
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
