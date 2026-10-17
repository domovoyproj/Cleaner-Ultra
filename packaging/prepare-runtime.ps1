$ErrorActionPreference = 'Stop'
$metadata = cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-gnu | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
$package = $metadata.packages | Where-Object name -EQ 'webview2-com-sys' | Select-Object -First 1
if (-not $package) { throw 'WebView2 loader package missing' }
$loader = Join-Path (Split-Path $package.manifest_path -Parent) 'x64\WebView2Loader.dll'
$loaderHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $loader).Hash
foreach ($directory in 'target\debug','target\debug\deps','target\release','target\release\deps') {
    New-Item -ItemType Directory -Force $directory | Out-Null
    $destination = Join-Path $directory 'WebView2Loader.dll'
    if ((Test-Path -LiteralPath $destination) -and (Get-FileHash -Algorithm SHA256 -LiteralPath $destination).Hash -eq $loaderHash) { continue }
    Copy-Item -LiteralPath $loader -Destination $directory -Force
}
