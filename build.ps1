# Build everything golden needs to run: the Rust workspace (core + server), in release mode.
# Usage: .\build.ps1          build
#        .\build.ps1 -Test    build, then run every test
param([switch]$Test)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

cargo build --workspace --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

if ($Test) {
    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { throw "tests failed" }
}

# web/ doesn't exist yet; once it does, its build goes here (npm ci; npm run build).
if (Test-Path "$PSScriptRoot\web\package.json") {
    Push-Location web
    try {
        npm ci; if ($LASTEXITCODE -ne 0) { throw "npm ci failed" }
        npm run build; if ($LASTEXITCODE -ne 0) { throw "web build failed" }
    } finally { Pop-Location }
}

Write-Host "Build OK. Start the server with .\run.ps1" -ForegroundColor Green
