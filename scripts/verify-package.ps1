[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArchivePath,
    [Parameter(Mandatory)]
    [ValidatePattern('^factory-droid\.(dll|so|dylib)$')]
    [string]$LibraryName
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$resolvedArchive = (Resolve-Path -LiteralPath $ArchivePath).Path
$archive = [IO.Compression.ZipFile]::OpenRead($resolvedArchive)
try {
    $entries = @($archive.Entries)
    $names = @($entries | ForEach-Object { $_.FullName })
    if (@($names | Select-Object -Unique).Count -ne $names.Count) {
        throw 'ZIP 包含重复文件名。'
    }
    foreach ($name in $names) {
        if ($name.Contains('\') -or $name.StartsWith('/') -or $name -match '(^|/)\.\.(/|$)') {
            throw "ZIP 包含非法路径：$name"
        }
        if ($name -match '(?i)(^|/)(temp|target|node_modules|\.git|\.env|auth\.json|accounts\.json)(/|$)' -or $name -match '(?i)\.(db|sqlite3?|key|pem)$') {
            throw "ZIP 包含运行数据、凭据文件或构建目录：$name"
        }
        if ($name -ne $LibraryName -and $name -notmatch '^(README\.md|LICENSE|SHA256SUMS\.txt|config\.example\.(yaml|json)|docs/.*)$') {
            throw "ZIP 包含未批准的文件：$name"
        }
    }
    $libraries = @($names | Where-Object { $_ -match '\.(dll|so|dylib)$' })
    if ($libraries.Count -ne 1 -or $libraries[0] -ne $LibraryName) {
        throw 'ZIP 必须包含唯一且位于根目录的目标动态库。'
    }
    foreach ($required in @('README.md', 'LICENSE', 'SHA256SUMS.txt')) {
        if ($names -notcontains $required) { throw "ZIP 缺少必要文件：$required" }
    }
    $readText = {
        param([string]$Name)
        $reader = [IO.StreamReader]::new($archive.GetEntry($Name).Open(), [Text.UTF8Encoding]::new($false, $true))
        try { $reader.ReadToEnd() } finally { $reader.Dispose() }
    }
    $libraryStream = $archive.GetEntry($LibraryName).Open()
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $libraryHash = [Convert]::ToHexString($hasher.ComputeHash($libraryStream)).ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
        $libraryStream.Dispose()
    }
    $checksum = (& $readText 'SHA256SUMS.txt').Trim()
    if ($checksum -ne "$libraryHash  $LibraryName") { throw 'ZIP 内动态库 SHA-256 不匹配。' }
    foreach ($entry in $entries | Where-Object { $_.FullName -match '\.(md|txt|json|ya?ml)$' }) {
        $text = & $readText $entry.FullName
        if ($text -match 'fk-[A-Za-z0-9_-]{24,}' -or $text -match '-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----') {
            throw "ZIP 文本文档包含完整凭据模式：$($entry.FullName)"
        }
    }
    $readme = & $readText 'README.md'
    foreach ($link in [regex]::Matches($readme, '\]\(([^():#]+\.md)(?:#[^)]*)?\)')) {
        if ($names -notcontains $link.Groups[1].Value) { throw "README 引用的本地文档未打包：$($link.Groups[1].Value)" }
    }
    $build = Get-Content -LiteralPath "$resolvedArchive.build.json" -Raw | ConvertFrom-Json
    $archiveHash = (Get-FileHash -LiteralPath $resolvedArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($build.archive.sha256 -ne $archiveHash -or $build.library.sha256 -ne $libraryHash) {
        throw '构建清单与 ZIP 或动态库 SHA-256 不匹配。'
    }
    Write-Output "安装包检查通过：$($names.Count) 个条目，唯一根动态库、文档链接、凭据模式、DLL/ZIP SHA-256 一致。"
}
finally {
    $archive.Dispose()
}
