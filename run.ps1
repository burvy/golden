# Start the golden server on http://localhost:8787 (Ctrl+C to stop).
# Builds first if needed, so this works on a fresh clone too.
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

cargo run --release -p server
if ($LASTEXITCODE -ne 0) { throw "server exited with code $LASTEXITCODE" }
