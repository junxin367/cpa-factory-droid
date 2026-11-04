[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Profile = 'release',
    [ValidateSet('', 'x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu', 'x86_64-apple-darwin', 'aarch64-apple-darwin')]
    [string]$Target = '',
    [switch]$ReleaseAssets
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$stagePath = $null
Push-Location $projectRoot
try {
    if ($ReleaseAssets -and $Profile -ne 'release') {
        throw '发布产物必须使用 release 配置。'
    }
    if (-not $Target) {
        $rustVersion = & rustc -vV
        if ($LASTEXITCODE -ne 0) { throw "rustc -vV 失败：$LASTEXITCODE" }
        $Target = ($rustVersion | Select-String -Pattern '^host: (.+)$').Matches.Groups[1].Value
    }
    $platforms = @{
        'x86_64-pc-windows-msvc' = @{ OS = 'windows'; Arch = 'amd64'; Extension = 'dll' }
        'x86_64-unknown-linux-gnu' = @{ OS = 'linux'; Arch = 'amd64'; Extension = 'so' }
        'x86_64-apple-darwin' = @{ OS = 'darwin'; Arch = 'amd64'; Extension = 'dylib' }
        'aarch64-apple-darwin' = @{ OS = 'darwin'; Arch = 'arm64'; Extension = 'dylib' }
    }
    if (-not $platforms.ContainsKey($Target)) {
        throw "暂不支持构建目标：$Target"
    }
    $platform = $platforms[$Target]
    $metadataJson = & cargo metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata 失败：$LASTEXITCODE" }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    $packages = @($metadata.packages | Where-Object { $_.name -eq 'cpa-factory-droid' })
    if ($packages.Count -ne 1) { throw 'Cargo.toml 必须包含唯一的 cpa-factory-droid 包。' }
    $package = $packages[0]
    if (-not @($package.targets | Where-Object { $_.crate_types -contains 'cdylib' }).Count) {
        throw 'cpa-factory-droid 必须配置 cdylib 动态库。'
    }
    $releaseScript = Join-Path $PSScriptRoot 'release.mjs'
    & node $releaseScript registry
    if ($LASTEXITCODE -ne 0) { throw '插件注册表校验失败。' }
    $sourceBefore = & node $releaseScript snapshot
    if ($LASTEXITCODE -ne 0) { throw '读取构建前源码指纹失败。' }

    $buildArgs = @('build', '--locked', '--package', 'cpa-factory-droid', '--lib', '--target', $Target, '--message-format=json-render-diagnostics')
    if ($Profile -eq 'release') { $buildArgs += '--release' }
    $buildMessages = & cargo @buildArgs
    $buildExit = $LASTEXITCODE
    $libraryFiles = @(
        foreach ($message in $buildMessages) {
            $entry = $message | ConvertFrom-Json
            if ($entry.reason -eq 'compiler-message' -and $entry.message.rendered) {
                Write-Host $entry.message.rendered
            }
            if ($entry.reason -eq 'compiler-artifact' -and $entry.package_id -eq $package.id -and $entry.target.crate_types -contains 'cdylib') {
                $entry.filenames | Where-Object { [IO.Path]::GetExtension($_) -eq ".$($platform.Extension)" }
            }
        }
    )
    if ($buildExit -ne 0) { throw "cargo build 失败：$buildExit" }
    if ($libraryFiles.Count -ne 1) { throw '无法从 Cargo 构建输出定位唯一的插件动态库。' }

    $distPath = Join-Path $projectRoot 'release'
    $stagingBase = Join-Path $projectRoot 'temp/packaging'
    New-Item -ItemType Directory -Path $distPath, $stagingBase -Force | Out-Null
    $stagePath = Join-Path $stagingBase ([Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stagePath | Out-Null
    $installedName = "factory-droid.$($platform.Extension)"
    Copy-Item -LiteralPath $libraryFiles[0] -Destination (Join-Path $stagePath $installedName)
    foreach ($requiredFile in @('README.md', 'LICENSE')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $requiredFile) -Destination $stagePath
    }
    foreach ($optionalFile in @('config.example.yaml', 'config.example.json')) {
        $optionalPath = Join-Path $projectRoot $optionalFile
        if (Test-Path -LiteralPath $optionalPath -PathType Leaf) {
            Copy-Item -LiteralPath $optionalPath -Destination $stagePath
        }
    }
    $docsPath = Join-Path $projectRoot 'docs'
    if (Test-Path -LiteralPath $docsPath -PathType Container) {
        Copy-Item -LiteralPath $docsPath -Destination (Join-Path $stagePath 'docs') -Recurse
    }
    $libraryHash = (Get-FileHash -LiteralPath (Join-Path $stagePath $installedName) -Algorithm SHA256).Hash.ToLowerInvariant()
    "$libraryHash  $installedName`n" | Set-Content -LiteralPath (Join-Path $stagePath 'SHA256SUMS.txt') -Encoding utf8NoBOM -NoNewline
    $suffix = if ($Profile -eq 'debug') { '_debug' } else { '' }
    $assetName = "factory-droid_$($package.version)_$($platform.OS)_$($platform.Arch)$suffix.zip"
    $archivePath = Join-Path $distPath $assetName
    $sourceAfter = & node $releaseScript snapshot
    if ($LASTEXITCODE -ne 0) { throw '读取构建后源码指纹失败。' }
    if ($sourceAfter -ne $sourceBefore) { throw '构建期间源码发生变化，需等待修改稳定后重新构建。' }
    Compress-Archive -Path (Join-Path $stagePath '*') -DestinationPath $archivePath -Force
    & node $releaseScript local $archivePath $Target $sourceBefore $libraryHash
    if ($LASTEXITCODE -ne 0) { throw '生成本地构建清单失败。' }
    & (Join-Path $PSScriptRoot 'verify-package.ps1') -ArchivePath $archivePath -LibraryName $installedName
    Write-Output "插件安装包：$archivePath"
}
finally {
    if ($stagePath -and (Test-Path -LiteralPath $stagePath)) {
        $resolvedStage = (Resolve-Path -LiteralPath $stagePath).Path
        $resolvedBase = (Resolve-Path -LiteralPath (Join-Path $projectRoot 'temp/packaging')).Path
        if (-not $resolvedStage.StartsWith($resolvedBase + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "拒绝清理预期目录外的临时文件：$resolvedStage"
        }
        # Windows scanners can briefly hold a freshly packed DLL open. Retry
        # only this verified staging directory; persistent failures still fail
        # the build, rather than reporting a successful cleanup.
        for ($cleanupAttempt = 0; $cleanupAttempt -lt 8; $cleanupAttempt++) {
            try {
                Remove-Item -LiteralPath $resolvedStage -Recurse -Force -ErrorAction Stop
                break
            }
            catch {
                if (-not (Test-Path -LiteralPath $resolvedStage)) { break }
                if ($cleanupAttempt -eq 7 -or -not $IsWindows) { throw }
                Start-Sleep -Milliseconds 250
            }
        }
    }
    Pop-Location
}
