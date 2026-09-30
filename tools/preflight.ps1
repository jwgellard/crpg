# Local native preflight: runs the repository's existing gates in a fixed
# order and stops at the first failure (T031). It provisions nothing, installs
# nothing, and never passes --target. See tools/README.md.
#
# Usage: pwsh -NoProfile -File tools/preflight.ps1 [-Crate <package>] [-CheckOnly]
#
# Exit codes: 0 all gates passed; 1 a gate failed (the command and its exit
# code are printed); 2 invalid arguments, missing prerequisite, or malformed
# cargo metadata (no modifying gate has run).
#
# Arguments are parsed by hand from $args (no param block) so that unknown,
# duplicate, and empty arguments reliably exit 2 instead of PowerShell's own
# binding error code.

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

function Show-Usage {
    @'
Usage: pwsh -NoProfile -File tools/preflight.ps1 [-Crate <package>] [-CheckOnly]

Runs the local native gates, in order, stopping at the first failure:
  1. rustc -vV
  2. cargo metadata package validation
  3. cargo fmt --all (skipped with -CheckOnly), then cargo fmt --all -- --check
  4. cargo clippy (workspace, or -p <package>) --all-targets --locked -- -D warnings
  5. cargo test (workspace, or -p <package>) --locked
  6. python tools/lint/deps.py
  7. python tools/lint/determinism.py
  8. python -m unittest discover -s tools/lint -p "test_*.py"
  9. cargo deny check (always the workspace graph)
 10. git diff --check

Options:
  -Crate <package>  run crate gates for one workspace member package
  -CheckOnly        do not rewrite formatting; only check it
  -Help             show this help and exit

A crate run does not satisfy the full-workspace or second-native-target
acceptance of any task; both modes are local checks, not a CI result.
'@ | Write-Output
}

function Stop-Usage([string]$Message) {
    [Console]::Error.WriteLine("preflight: $Message")
    [Console]::Error.WriteLine('Run with -Help for usage.')
    exit 2
}

function Stop-Prerequisite([string]$Message) {
    [Console]::Error.WriteLine("preflight: $Message")
    exit 2
}

$crate = $null
$checkOnly = $false
$index = 0
while ($index -lt $args.Count) {
    $argument = [string]$args[$index]
    switch -CaseSensitive ($argument) {
        { $_ -in @('-Help', '-help', '--help', '-h') } {
            Show-Usage
            exit 0
        }
        { $_ -in @('-Crate', '-crate') } {
            if ($null -ne $crate) { Stop-Usage '-Crate given more than once' }
            if ($index + 1 -ge $args.Count) { Stop-Usage '-Crate needs a package name' }
            $value = [string]$args[$index + 1]
            if ([string]::IsNullOrEmpty($value)) { Stop-Usage '-Crate package name is empty' }
            $crate = $value
            $index += 2
            continue
        }
        { $_ -in @('-CheckOnly', '-checkonly') } {
            if ($checkOnly) { Stop-Usage '-CheckOnly given more than once' }
            $checkOnly = $true
            $index += 1
            continue
        }
        default {
            Stop-Usage "unexpected argument: $argument"
        }
    }
}

$root = Split-Path -Parent $PSScriptRoot
foreach ($required in @('Cargo.toml', 'rust-toolchain.toml', 'tools/lint/deps.py', 'tools/lint/determinism.py')) {
    if (-not (Test-Path -LiteralPath (Join-Path $root $required) -PathType Leaf)) {
        Stop-Prerequisite "missing $required under $root"
    }
}

$tools = @{}
foreach ($name in @('rustc', 'cargo', 'python', 'git')) {
    $command = Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($null -eq $command) {
        Stop-Prerequisite "required executable not found on PATH: $name"
    }
    $tools[$name] = $command.Source
}

# Runs one gate, passing its output through; stops on a nonzero exit.
function Invoke-Gate([string]$Name, [string[]]$Arguments) {
    Write-Output ("==> $Name " + ($Arguments -join ' '))
    & $tools[$Name] @Arguments | Out-Host
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        [Console]::Error.WriteLine("preflight: FAILED (exit $code): $Name " + ($Arguments -join ' '))
        exit 1
    }
}

Push-Location -LiteralPath $root
try {
    & $tools['python'] -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)'
    if ($LASTEXITCODE -ne 0) {
        Stop-Prerequisite 'python 3.11 or newer is required'
    }

    Invoke-Gate 'rustc' @('-vV')

    $metadataArgs = @('metadata', '--no-deps', '--format-version', '1', '--locked')
    Write-Output ('==> cargo ' + ($metadataArgs -join ' '))
    $metadataText = (& $tools['cargo'] @metadataArgs) -join "`n"
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        [Console]::Error.WriteLine("preflight: FAILED (exit $code): cargo " + ($metadataArgs -join ' '))
        exit 1
    }
    try {
        $metadata = $metadataText | ConvertFrom-Json
        $members = @($metadata.workspace_members)
        $packages = @($metadata.packages)
        if ($members.Count -eq 0) { throw 'no workspace members' }
        $names = @($packages | Where-Object { $members -contains $_.id } | ForEach-Object { [string]$_.name })
        if ($names.Count -ne $members.Count) { throw 'workspace member without a package entry' }
    }
    catch {
        Stop-Prerequisite "malformed cargo metadata: $($_.Exception.Message)"
    }
    if ($null -ne $crate -and -not ($names -ccontains $crate)) {
        Stop-Prerequisite "'$crate' is not a workspace member package"
    }

    if (-not $checkOnly) {
        Invoke-Gate 'cargo' @('fmt', '--all')
    }
    Invoke-Gate 'cargo' @('fmt', '--all', '--', '--check')
    if ($null -ne $crate) {
        Invoke-Gate 'cargo' @('clippy', '-p', $crate, '--all-targets', '--locked', '--', '-D', 'warnings')
        Invoke-Gate 'cargo' @('test', '-p', $crate, '--locked')
    }
    else {
        Invoke-Gate 'cargo' @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
        Invoke-Gate 'cargo' @('test', '--workspace', '--locked')
    }
    Invoke-Gate 'python' @('tools/lint/deps.py')
    Invoke-Gate 'python' @('tools/lint/determinism.py')
    Invoke-Gate 'python' @('-m', 'unittest', 'discover', '-s', 'tools/lint', '-p', 'test_*.py')
    Invoke-Gate 'cargo' @('deny', 'check')
    Invoke-Gate 'git' @('diff', '--check')

    if ($null -ne $crate) {
        Write-Output "preflight: all crate gates passed for $crate (not a full-workspace result)"
    }
    else {
        Write-Output 'preflight: all workspace gates passed'
    }
    exit 0
}
finally {
    Pop-Location
}
