param([switch]$BuildOnly, [string]$DataPath)
$ErrorActionPreference='Stop'
try {
    $root=Split-Path $PSScriptRoot -Parent
    Set-Location -LiteralPath $root
    $manifest=Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
    if ($manifest -notmatch '(?m)^version = "(\d+\.\d+\.\d+)"') { throw 'Version missing from Cargo.toml' }
    $releaseVersion=$Matches[1]
    $exe=Join-Path $root "dist\CleanerUltra-$releaseVersion\CleanerUltra.exe"
    if (-not (Test-Path -LiteralPath $exe)) {
        $exe=Join-Path $root 'target\release\cleaner_ultra.exe'
        & cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Cleaner Ultra build failed' }
        & (Join-Path $PSScriptRoot 'prepare-runtime.ps1')
    }
    if (-not $BuildOnly) {
        $startOptions=@{ FilePath=$exe; WorkingDirectory=$root; WindowStyle='Hidden' }
        if ($DataPath) {
            if ($DataPath.Contains('"')) { throw 'Invalid data directory' }
            $startOptions.ArgumentList=@('--data', ('"' + [IO.Path]::GetFullPath($DataPath) + '"'))
        }
        Start-Process @startOptions | Out-Null
    }
    exit 0
} catch {
    if (-not $BuildOnly) {
        Add-Type -AssemblyName System.Windows.Forms
        [System.Windows.Forms.MessageBox]::Show("Не удалось запустить Cleaner Ultra: $($_.Exception.Message)", 'Cleaner Ultra') | Out-Null
    }
    Write-Error $_
    exit 1
}
