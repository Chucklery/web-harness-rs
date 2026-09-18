param(
    [Parameter(Mandatory = $true)]
    [string]$Target,
    [Parameter(Mandatory = $true)]
    [string]$Destination
)

$ErrorActionPreference = "Stop"
$Version = "0.0.14"
$PrimaryBaseUrl = "https://persistent.oaistatic.com/tunnel-client/v$Version"
$FallbackBaseUrl = "https://github.com/openai/tunnel-client/releases/download/v$Version"

switch ($Target) {
    "x86_64-pc-windows-msvc" {
        $Platform = "windows-amd64"
        $ExpectedSha256 = "784ab8da7b5a88f0109f1fd8aaf0a1c86067430b896dddf307ef7e3cc49fa1a5"
    }
    "aarch64-pc-windows-msvc" {
        $Platform = "windows-arm64"
        $ExpectedSha256 = "fa775db8897df543dd4ba66404f69492a2acfbc6a291f10df27aced064a16568"
    }
    default {
        throw "unsupported tunnel-client Windows target: $Target"
    }
}

$Archive = "tunnel-client-v$Version-$Platform.zip"
$PrimaryUrl = "$PrimaryBaseUrl/$Archive"
$FallbackUrl = "$FallbackBaseUrl/$Archive"
$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("web-harness-tunnel-" + [guid]::NewGuid())
$ArchivePath = Join-Path $TempDir $Archive

New-Item -ItemType Directory -Path $TempDir -Force | Out-Null
try {
    try {
        Invoke-WebRequest -Uri $PrimaryUrl -OutFile $ArchivePath -UseBasicParsing
    }
    catch {
        Write-Warning "primary OpenAI CDN download failed; trying GitHub release fallback"
        Invoke-WebRequest -Uri $FallbackUrl -OutFile $ArchivePath -UseBasicParsing
    }

    $ActualSha256 = (Get-FileHash -Algorithm SHA256 $ArchivePath).Hash.ToLowerInvariant()
    if ($ActualSha256 -ne $ExpectedSha256) {
        throw "tunnel-client checksum mismatch for $Archive; expected $ExpectedSha256, actual $ActualSha256"
    }

    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    Expand-Archive -Path $ArchivePath -DestinationPath $Destination -Force
    Set-Content -Path (Join-Path $Destination "UPSTREAM_VERSION") -Value "v$Version" -NoNewline
    Set-Content -Path (Join-Path $Destination "UPSTREAM_URL") -Value $PrimaryUrl -NoNewline
    Set-Content -Path (Join-Path $Destination "UPSTREAM_FALLBACK_URL") -Value $FallbackUrl -NoNewline

    Write-Output (Join-Path $Destination "tunnel-client.exe")
}
finally {
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
