<#
.SYNOPSIS
  Phase 1 of LBL_V18_BENCHMARK_PLAN.md: writes a large synthetic build with each layout, reads it back, and records peak memory.

.DESCRIPTION
  Fill phase (default): for each layout, write a sealed .lbl of -TargetGb, run the read queries (with a full channel scan), delete the file.
  Realtime phase (-Realtime): write at the real-time rate for -RealtimeMinutes with the writer paced like a capture, to measure latency under load.
  Results go to -Results as JSON lines, one file per step. Build first: cargo build --release -p build-log-io --example layout_bench
  Needs about 1.3 x TargetGb of free disk. The data is synthetic.
#>
param(
    [string]$Work = "$env:TEMP\lb_work",
    [string]$Results = "$env:TEMP\lb_results",
    [double]$TargetGb = 50,
    [string]$Filters = 'gzip3shufflef32',
    [string[]]$Layouts = @('build_wide', 'per_layer'),
    [switch]$Realtime,
    [int]$RealtimeMinutes = 30,
    # keep a file that an earlier run already wrote (and its write result) and carry on with the read step
    [switch]$Resume,
    # tile laser layers from a real V17 build (see real.rs) instead of synthetic data
    [string]$Real = '',
    # store coordinates delta + zigzag encoded (codec.rs)
    [switch]$Encode
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..\target\release\examples\layout_bench.exe')).Path
New-Item -ItemType Directory -Force $Work, $Results | Out-Null
$extra = @()
if ($Real) { $extra += @('--real', $Real) }
if ($Encode) { $extra += '--encode' }

function Invoke-Measured([string]$Name, [string[]]$ToolArgs) {
    $out = Join-Path $Results "$Name.jsonl"
    $err = Join-Path $Results "$Name.log"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $exe -ArgumentList $ToolArgs -RedirectStandardOutput $out -RedirectStandardError $err -PassThru -NoNewWindow
    $null = $p.Handle # caching the handle is what makes ExitCode available after the process ends
    $peak = 0
    while (-not $p.HasExited) {
        try { $p.Refresh(); $peak = [Math]::Max($peak, $p.PeakWorkingSet64) } catch { }
        Start-Sleep -Seconds 5
    }
    $sw.Stop()
    $p.WaitForExit() # without this, ExitCode can be empty after polling HasExited
    $meta = [ordered]@{ step = $Name; exit_code = $p.ExitCode; wall_s = [int]$sw.Elapsed.TotalSeconds; peak_working_set_mb = [int]($peak / 1MB); filters = $Filters; args = ($ToolArgs -join ' ') }
    ($meta | ConvertTo-Json -Compress) | Add-Content (Join-Path $Results "$Name.meta.json")
    if ($p.ExitCode -ne 0) { throw "$Name failed with exit code $($p.ExitCode); see $err" }
}

foreach ($layout in $Layouts) {
    $file = Join-Path $Work "$layout.lbl"
    $skipWrite = $Resume -and (Test-Path $file) -and ((Get-Item (Join-Path $Results "$layout.write.jsonl") -ErrorAction SilentlyContinue).Length -gt 0)
    if ((Test-Path $file) -and -not $skipWrite) { Remove-Item $file -Force }
    $free = (Get-PSDrive -Name ([IO.Path]::GetPathRoot($Work).Substring(0, 1))).Free / 1GB
    if ($free -lt $TargetGb * 1.3 -and -not $skipWrite) { throw "only $([int]$free) GB free; need about $([int]($TargetGb * 1.3)) GB" }

    if ($Realtime) {
        $layers = [int]($RealtimeMinutes * 6) # 10-second layers
        Invoke-Measured "$layout.realtime" @('write', '--layout', $layout, '--path', $file, '--realtime', '--max-layers', $layers, '--target-gb', 9999, '--filters', $Filters, '--no-seal')
    }
    else {
        if (-not $skipWrite) { Invoke-Measured "$layout.write" (@('write', '--layout', $layout, '--path', $file, '--target-gb', $TargetGb, '--filters', $Filters) + $extra) }
        Invoke-Measured "$layout.read" @('read', '--path', $file, '--repeat', 3, '--scan')
    }
    Remove-Item $file -Force
}
Write-Output "done: $Results"
