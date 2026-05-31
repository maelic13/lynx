param(
    [string]$Base = "target\regression\rarog-baseline.exe",
    [string]$Candidate = "target\release\rarog.exe",
    [int]$Games = 64,
    [int]$Concurrency = 24,
    [string]$TimeControl = "0.6+0.006",
    [int]$Hash = 64,
    [int]$Threads = 1,
    [int]$TimeMargin = 0,
    [int]$MaxMoves = 0,
    [switch]$NoScoreAdjudication,
    [string]$Cutechess = "D:\chess\cutechess-cli\cutechess-cli.exe",
    [string]$Book = "D:\chess\books\Perfect2023.bin",
    [string]$Openings = "",
    [string]$OpeningsFormat = "pgn",
    [string]$OpeningsOrder = "random",
    [int]$OpeningsPlies = 8,
    [string]$OpeningsPolicy = "round",
    [switch]$NoRepeat,
    [int]$Seed = 0,
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
if ($Openings -eq "" -and -not (Test-Path -LiteralPath $Book)) {
    throw "opening book not found at $Book"
}
if ($Openings -ne "" -and -not (Test-Path -LiteralPath $Openings)) {
    throw "openings file not found at $Openings"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$basePath = (Resolve-Path -LiteralPath $Base).Path
$candidatePath = (Resolve-Path -LiteralPath $Candidate).Path
$outPath = (Resolve-Path -LiteralPath $OutDir).Path
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$pgn = Join-Path $outPath "rarog-regression-$stamp.pgn"
$log = Join-Path $outPath "rarog-regression-$stamp.log"
$gamesPerRound = if ($NoRepeat) { 1 } else { 2 }
$rounds = if ($NoRepeat) {
    [Math]::Max(1, $Games)
} else {
    [Math]::Max(1, [Math]::Ceiling($Games / 2.0))
}
$scheduledGames = $rounds * $gamesPerRound

$eachArgs = @("tc=$TimeControl")
if ($TimeMargin -gt 0) {
    $eachArgs += "timemargin=$TimeMargin"
}
if ($Openings -eq "") {
    $bookPath = (Resolve-Path -LiteralPath $Book).Path
    $eachArgs += @("book=$bookPath", "bookdepth=8")
}

$args = @(
    "-engine", "name=Rarog-base", "cmd=$basePath", "proto=uci", "option.Threads=$Threads", "option.Hash=$Hash",
    "-engine", "name=Rarog-test", "cmd=$candidatePath", "proto=uci", "option.Threads=$Threads", "option.Hash=$Hash",
    "-tournament", "round-robin",
    "-each"
) + $eachArgs + @(
    "-rounds", "$rounds",
    "-games", "$gamesPerRound",
    "-concurrency", "$Concurrency",
    "-pgnout", $pgn,
    "-recover",
    "-ratinginterval", "8"
)

if (-not $NoRepeat) {
    $args += "-repeat"
}
if (-not $NoScoreAdjudication) {
    $args += @(
        "-draw", "movenumber=40", "movecount=15", "score=5",
        "-resign", "movecount=3", "score=1500"
    )
}
if ($MaxMoves -gt 0) {
    $args += @("-maxmoves", "$MaxMoves")
}

if ($Openings -ne "") {
    $openingPath = (Resolve-Path -LiteralPath $Openings).Path
    $args += @(
        "-openings",
        "file=$openingPath",
        "format=$OpeningsFormat",
        "order=$OpeningsOrder",
        "policy=$OpeningsPolicy"
    )
    if ($OpeningsFormat -ieq "pgn") {
        $args += "plies=$OpeningsPlies"
    }
} else {
    $bookPath = (Resolve-Path -LiteralPath $Book).Path
}

if ($Seed -ne 0) {
    $args += @("-srand", "$Seed")
}

Write-Host "Running $scheduledGames games at tc=$TimeControl, concurrency=$Concurrency"
Write-Host "Base:      $basePath"
Write-Host "Candidate: $candidatePath"
if ($Openings -ne "") {
    $pliesText = if ($OpeningsFormat -ieq "pgn") { ", $OpeningsPlies plies" } else { "" }
    Write-Host "Openings:  $openingPath ($OpeningsFormat, $OpeningsOrder$pliesText)"
} else {
    Write-Host "Book:      $bookPath"
}
if ($NoRepeat) {
    Write-Host "Pairing:   unpaired starts (Little Blitzer-style)"
} else {
    Write-Host "Pairing:   repeated colors"
}
if ($Seed -ne 0) {
    Write-Host "Seed:      $Seed"
}
Write-Host "Log:       $log"
Write-Host "PGN:       $pgn"

& $Cutechess @args 2>&1 | Tee-Object -FilePath $log
exit $LASTEXITCODE
