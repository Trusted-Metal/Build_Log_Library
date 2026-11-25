<#
.SYNOPSIS
  W10 of LBL_V18_BENCHMARK_PLAN.md: kill the writer at random moments (200 kills over five configurations) and kill the seal.

.DESCRIPTION
  Configurations: build_wide and per_layer, each with no H5Fflush and with one per layer; build_wide also with a partial-chunk flush
  of every sensor at each layer end. Each run writes a synthetic build, kills the process 2 to 20 s in, then opens what is left,
  regenerates what should be there and compares. Results (one summary line, then one line per run) go to -Results.
  Build first: cargo build --release -p build-log-io --example layout_bench
#>
param(
    [string]$Results = "$env:TEMP\lb_results_w10",
    [int]$Runs = 40,
    [int]$SealRuns = 20,
    [double]$SealBaseGb = 2
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..\target\release\examples\layout_bench.exe')).Path
New-Item -ItemType Directory -Force $Results | Out-Null

function Invoke-Step([string]$Name, [string[]]$ToolArgs) {
    $p = Start-Process $exe -ArgumentList $ToolArgs -RedirectStandardOutput (Join-Path $Results "$Name.jsonl") -RedirectStandardError (Join-Path $Results "$Name.log") -PassThru -NoNewWindow
    $null = $p.Handle
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) { throw "$Name failed with exit code $($p.ExitCode); see $Results\$Name.log" }
}

$configs = @(
    @{ Name = 'build_wide_noflush';      Layout = 'build_wide'; Flush = 0; Tail = 0 },
    @{ Name = 'build_wide_flush';        Layout = 'build_wide'; Flush = 1; Tail = 0 },
    @{ Name = 'build_wide_flush_tail';   Layout = 'build_wide'; Flush = 1; Tail = 1 },
    @{ Name = 'per_layer_noflush';       Layout = 'per_layer';  Flush = 0; Tail = 0 },
    @{ Name = 'per_layer_flush';         Layout = 'per_layer';  Flush = 1; Tail = 0 }
)
foreach ($c in $configs) {
    Invoke-Step "w10_$($c.Name)" @('kill-test', '--layout', $c.Layout, '--runs', $Runs, '--flush-layers', $c.Flush, '--tail-flush-layers', $c.Tail, '--layer-seconds', 3, '--dir', (Join-Path $env:TEMP "lb_kill_$($c.Name)"))
}
Invoke-Step 'w10b_seal_kill' @('seal-kill', '--runs', $SealRuns, '--base-gb', $SealBaseGb, '--dir', (Join-Path $env:TEMP 'lb_sealkill'))
Write-Output "done: $Results"
