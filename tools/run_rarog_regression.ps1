param(
    [string]$Base = "target\regression\rarog-baseline.exe",
    [string]$Candidate = "target\release\rarog.exe",
    [int]$Games = 64,
    [int]$Concurrency = 24,
    [string]$TimeControl = "0.6+0.006",
    [int]$Hash = 64,
    [int]$Threads = 1,
    [string]$Cutechess = "D:\chess\cutechess-cli\cutechess-cli.exe",
    [string]$Book = "D:\chess\books\Perfect2023.bin",
    [string]$OutDir = "target\regression\results"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Cutechess)) {
    throw "cutechess-cli not found at $Cutechess"
}
if (-not (Test-Path -LiteralPath $Base)) {
    throw "baseline engine not found at $Base"
}
if (-not (Test-Path -LiteralPath $Candidate)) {
    throw "candidate engine not found at $Candidate"
}
if (-not (Test-Path -LiteralPath $Book)) {
    throw "opening book not found at $Book"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$basePath = (Resolve-Path -LiteralPath $Base).Path
$candidatePath = (Resolve-Path -LiteralPath $Candidate).Path
$outPath = (Resolve-Path -LiteralPath $OutDir).Path
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$pgn = Join-Path $outPath "rarog-regression-$stamp.pgn"
$log = Join-Path $outPath "rarog-regression-$stamp.log"
$rounds = [Math]::Max(1, [Math]::Ceiling($Games / 2.0))

$args = @(
    "-engine", "name=Rarog-base", "cmd=$basePath", "proto=uci", "option.Threads=$Threads", "option.Hash=$Hash",
    "-engine", "name=Rarog-test", "cmd=$candidatePath", "proto=uci", "option.Threads=$Threads", "option.Hash=$Hash",
    "-tournament", "round-robin",
    "-each", "tc=$TimeControl", "book=$Book", "bookdepth=8",
    "-rounds", "$rounds",
    "-games", "2",
    "-repeat",
    "-concurrency", "$Concurrency",
    "-pgnout", $pgn,
    "-recover",
    "-draw", "movenumber=40", "movecount=15", "score=5",
    "-resign", "movecount=3", "score=1500",
    "-ratinginterval", "8"
)

Write-Host "Running $($rounds * 2) games at tc=$TimeControl, concurrency=$Concurrency"
Write-Host "Base:      $basePath"
Write-Host "Candidate: $candidatePath"
Write-Host "Log:       $log"
Write-Host "PGN:       $pgn"

& $Cutechess @args 2>&1 | Tee-Object -FilePath $log
exit $LASTEXITCODE
