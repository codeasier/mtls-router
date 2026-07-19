$ErrorActionPreference = 'Stop'

[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Repo = 'codeasier/mtls-router'
$ScriptDir = $PSScriptRoot
$UserHome = if ($env:USERPROFILE) { $env:USERPROFILE } elseif ($HOME) { $HOME } else { throw '无法确定用户主目录。' }
$InstallDir = if ($env:MTLS_ROUTER_INSTALL_DIR) { $env:MTLS_ROUTER_INSTALL_DIR } else { Join-Path $UserHome '.local\bin' }
$StateDir = if ($env:MTLS_ROUTER_STATE_DIR) { $env:MTLS_ROUTER_STATE_DIR } else { Join-Path $UserHome '.mtls-router' }
$DefaultDownloadBaseUrl = ''
$DownloadBaseUrl = if ($env:MTLS_ROUTER_DOWNLOAD_URL) { $env:MTLS_ROUTER_DOWNLOAD_URL } else { $DefaultDownloadBaseUrl }
$DownloadUser = $env:MTLS_ROUTER_DOWNLOAD_USER
$DownloadPassword = $env:MTLS_ROUTER_DOWNLOAD_PASSWORD
$AllowDownload = $env:MTLS_ROUTER_ALLOW_DOWNLOAD -eq '1'
$RouterPath = Join-Path $InstallDir 'mtls-router.exe'
$ManagerPath = Join-Path $InstallDir 'mtls-router-manager.exe'
$ReceiptPath = Join-Path $StateDir 'install-receipt.json'
$PendingPath = Join-Path $StateDir 'install-pending.json'
$BackupDir = Join-Path $StateDir 'install-previous'
$RouterBaseUrl = 'http://127.0.0.1:19099'
$ManagementProtocolVersion = '2'
$MaxModelConfigSize = 2 * 1024 * 1024
$script:TransientApiKey = ''
$script:TransientRequest = ''

function Write-Info($Message) { Write-Host $Message -ForegroundColor Cyan }
function Write-Success($Message) { Write-Host $Message -ForegroundColor Green }
function Write-Warn($Message) { Write-Host $Message -ForegroundColor Yellow }
function Write-Fail($Message) { Write-Host $Message -ForegroundColor Red -ErrorAction Continue; exit 1 }

function Show-Banner {
    Write-Info '============================================================'
    Write-Info ' mtls-router 代理配置向导'
    Write-Info '============================================================'
}

function Get-PlatformAssets {
    $arch = if (($env:PROCESSOR_ARCHITECTURE -match 'ARM64') -or ($env:PROCESSOR_ARCHITEW6432 -match 'ARM64')) { 'arm64' }
        elseif ($env:PROCESSOR_ARCHITECTURE -match 'AMD64|x86_64') { 'amd64' }
        else { Write-Fail "不支持的 CPU 架构：$env:PROCESSOR_ARCHITECTURE" }
    return [pscustomobject]@{
        Router = "mtls-router-windows-$arch.exe"
        Manager = "mtls-router-manager-windows-$arch.exe"
    }
}

function Get-ExpectedChecksum($ManifestPath, $Asset) {
    $escaped = [regex]::Escape($Asset)
    $candidates = @(Get-Content -LiteralPath $ManifestPath | Where-Object { $_ -match "[\s\*]$escaped$" })
    if ($candidates.Count -ne 1) { Write-Fail "SHA256SUMS 必须包含且仅包含一条 $Asset 校验候选记录。" }
    if ($candidates[0] -notmatch "^[0-9A-Fa-f]{64} ( |\*)$escaped$") { Write-Fail "SHA256SUMS 中的 $Asset 校验记录格式无效。" }
    return $candidates[0].Substring(0, 64).ToLowerInvariant()
}

function Get-Hash($Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Test-Checksum($SourcePath, $ManifestPath, $Asset) {
    if ((Get-Hash $SourcePath) -ne (Get-ExpectedChecksum $ManifestPath $Asset)) { Write-Fail "SHA-256 校验失败：$Asset" }
}

function Test-Hash($Path, $Expected) {
    return (Test-Path -LiteralPath $Path -PathType Leaf) -and ((Get-Hash $Path) -eq $Expected)
}

function Set-PrivatePath($Path, [bool]$Directory) {
    if (-not $IsWindows -and $PSVersionTable.PSVersion.Major -ge 6) {
        & chmod $(if ($Directory) { '700' } else { '600' }) $Path
        return
    }
    try {
        $acl = Get-Acl -LiteralPath $Path
        $acl.SetAccessRuleProtection($true, $false)
        foreach ($rule in @($acl.Access)) { $acl.RemoveAccessRuleAll($rule) }
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        $rights = if ($Directory) { [Security.AccessControl.FileSystemRights]::FullControl } else { [Security.AccessControl.FileSystemRights]::FullControl }
        $inheritance = if ($Directory) { [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit' } else { [Security.AccessControl.InheritanceFlags]::None }
        $rule = New-Object Security.AccessControl.FileSystemAccessRule($identity, $rights, $inheritance, [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Allow)
        $acl.AddAccessRule($rule)
        Set-Acl -LiteralPath $Path -AclObject $acl
    } catch { Write-Fail "无法限制安装状态权限：$Path" }
}

function Write-PrivateJson($Path, $Value) {
    $dir = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Set-PrivatePath $dir $true
    $tmp = Join-Path $dir ('.' + [IO.Path]::GetFileName($Path) + '.' + [Guid]::NewGuid().ToString('N'))
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value + [Environment]::NewLine)
        $stream = [IO.File]::Open($tmp, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
        Set-PrivatePath $tmp $false
        Move-Item -LiteralPath $tmp -Destination $Path -Force
        Set-PrivatePath $Path $false
    } finally { Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue }
}

function Test-Receipt {
    if (-not (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) { return $false }
    try { $receipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json } catch { return $false }
    if ($receipt.schema_version -ne 1 -or $receipt.router.path -cne $RouterPath -or $receipt.manager.path -cne $ManagerPath) { return $false }
    if (-not $receipt.deployment_id -or [string]$receipt.management_protocol_version -cne $ManagementProtocolVersion) { return $false }
    return (Test-Hash $RouterPath $receipt.router.sha256) -and (Test-Hash $ManagerPath $receipt.manager.sha256)
}

function Commit-Pending {
    $pending = Get-Content -LiteralPath $PendingPath -Raw | ConvertFrom-Json
    $receipt = [ordered]@{
        schema_version = 1
        deployment_id = $pending.deployment_id
        management_protocol_version = $pending.management_protocol_version
        installed_at = $pending.installed_at
        router = $pending.router
        manager = $pending.manager
    } | ConvertTo-Json -Depth 20
    Write-PrivateJson $ReceiptPath $receipt
    Remove-Item -LiteralPath $PendingPath -Force
    Remove-Item -LiteralPath $BackupDir -Recurse -Force -ErrorAction SilentlyContinue
}

function Restore-Previous {
    $pending = Get-Content -LiteralPath $PendingPath -Raw | ConvertFrom-Json
    if ($pending.previous.committed) {
        $routerBackup = Join-Path $BackupDir 'mtls-router.exe'
        $managerBackup = Join-Path $BackupDir 'mtls-router-manager.exe'
        if (-not (Test-Hash $routerBackup $pending.previous.router.sha256) -or -not (Test-Hash $managerBackup $pending.previous.manager.sha256)) { return $false }
        Copy-Item -LiteralPath $routerBackup -Destination (Join-Path $InstallDir '.mtls-router.restore') -Force
        Copy-Item -LiteralPath $managerBackup -Destination (Join-Path $InstallDir '.mtls-router-manager.restore') -Force
        Move-Item -LiteralPath (Join-Path $InstallDir '.mtls-router.restore') -Destination $RouterPath -Force
        Move-Item -LiteralPath (Join-Path $InstallDir '.mtls-router-manager.restore') -Destination $ManagerPath -Force
        if (-not (Test-Hash $RouterPath $pending.previous.router.sha256) -or -not (Test-Hash $ManagerPath $pending.previous.manager.sha256)) { return $false }
        Write-PrivateJson $ReceiptPath ($pending.previous.receipt | ConvertTo-Json -Depth 20)
    } else {
        Remove-Item -LiteralPath $RouterPath, $ManagerPath, $ReceiptPath -Force -ErrorAction SilentlyContinue
        if ((Test-Path $RouterPath) -or (Test-Path $ManagerPath)) { return $false }
    }
    Remove-Item -LiteralPath $PendingPath -Force
    Remove-Item -LiteralPath $BackupDir -Recurse -Force -ErrorAction SilentlyContinue
    return $true
}

function Repair-PendingInstall {
    if (-not (Test-Path -LiteralPath $PendingPath)) { return }
    try { $pending = Get-Content -LiteralPath $PendingPath -Raw | ConvertFrom-Json } catch { Write-Fail "安装事务标记损坏，拒绝执行 router 或 manager：$PendingPath" }
    if ($pending.schema_version -ne 1 -or $pending.router.path -cne $RouterPath -or $pending.manager.path -cne $ManagerPath) { Write-Fail '安装事务标记与固定安装路径不匹配，拒绝执行。' }
    if ((Test-Hash $RouterPath $pending.router.sha256) -and (Test-Hash $ManagerPath $pending.manager.sha256)) {
        Commit-Pending
        Write-Success '  已完成中断的 mtls-router/manager 安装事务。'
        return
    }
    if (-not (Restore-Previous)) { Write-Fail '无法证明新旧任一完整安装代，拒绝执行 router 或 manager。' }
    Write-Warn '  已回滚中断的 mtls-router/manager 安装事务。'
}

function Invoke-ManagerAt($Manager, $Router, $Request) {
    $response = $Request | & $Manager serve --router-sidecar $Router
    if ($LASTEXITCODE -ne 0) { Write-Fail 'mtls-router-manager 执行失败。' }
    $lines = @($response)
    if ($lines.Count -ne 1) { Write-Fail 'manager 返回了无效的多行响应。' }
    try { $decoded = $lines[0] | ConvertFrom-Json } catch { Write-Fail 'manager 返回了无效 JSON。' }
    if ($decoded.error) { Write-Fail "manager $($decoded.error.code): $($decoded.error.message)" }
    return $decoded
}

function Get-BinaryVersion($Path) {
    $output = & $Path --version 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $output) { Write-Fail "无法读取已验证二进制版本：$Path" }
    return ([string]$output).Split(' ')[-1]
}

function Install-Pair($SourceRouter, $SourceManager) {
    $routerHash = Get-Hash $SourceRouter
    $managerHash = Get-Hash $SourceManager
    $info = Invoke-ManagerAt $SourceManager $SourceRouter '{"id":"setup-info","method":"manager.info","params":{}}'
    if (-not $info.result.version -or -not $info.result.deployment_id -or [string]$info.result.management_protocol_version -cne $ManagementProtocolVersion) { Write-Fail 'manager 元数据响应无效。' }
    $routerVersion = Get-BinaryVersion $SourceRouter
    if ($routerVersion -cne [string]$info.result.version) { Write-Fail 'router 与 manager 版本不匹配，拒绝安装。' }
    New-Item -ItemType Directory -Force -Path $InstallDir, $StateDir | Out-Null
    Set-PrivatePath $StateDir $true
    Repair-PendingInstall

    Remove-Item -LiteralPath $BackupDir -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $BackupDir | Out-Null
    Set-PrivatePath $BackupDir $true
    $previous = $false
    $oldReceipt = [pscustomobject]@{}
    if ((Test-Path $RouterPath) -or (Test-Path $ManagerPath) -or (Test-Path $ReceiptPath)) {
        if (-not (Test-Receipt)) { Write-Fail '现有安装不是 receipt 验证的完整二进制对；拒绝部分覆盖。' }
        Copy-Item -LiteralPath $RouterPath -Destination (Join-Path $BackupDir 'mtls-router.exe')
        Copy-Item -LiteralPath $ManagerPath -Destination (Join-Path $BackupDir 'mtls-router-manager.exe')
        Set-PrivatePath (Join-Path $BackupDir 'mtls-router.exe') $false
        Set-PrivatePath (Join-Path $BackupDir 'mtls-router-manager.exe') $false
        $oldReceipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
        $previous = $true
    }
    $pending = [ordered]@{
        schema_version = 1
        transaction_id = [Guid]::NewGuid().ToString('N')
        installed_at = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
        deployment_id = $info.result.deployment_id
        management_protocol_version = $info.result.management_protocol_version
        router = [ordered]@{ path = $RouterPath; sha256 = $routerHash; version = $routerVersion }
        manager = [ordered]@{ path = $ManagerPath; sha256 = $managerHash; version = $info.result.version }
        previous = [ordered]@{
            committed = $previous
            receipt = $oldReceipt
            router = if ($previous) { $oldReceipt.router } else { [pscustomobject]@{} }
            manager = if ($previous) { $oldReceipt.manager } else { [pscustomobject]@{} }
        }
    } | ConvertTo-Json -Depth 20
    Write-PrivateJson $PendingPath $pending

    Copy-Item -LiteralPath $SourceRouter -Destination (Join-Path $InstallDir '.mtls-router.new') -Force
    Move-Item -LiteralPath (Join-Path $InstallDir '.mtls-router.new') -Destination $RouterPath -Force
    if ($env:MTLS_ROUTER_INSTALL_CRASH_POINT -eq 'after-router') { exit 97 }
    Copy-Item -LiteralPath $SourceManager -Destination (Join-Path $InstallDir '.mtls-router-manager.new') -Force
    Move-Item -LiteralPath (Join-Path $InstallDir '.mtls-router-manager.new') -Destination $ManagerPath -Force
    if ($env:MTLS_ROUTER_INSTALL_CRASH_POINT -eq 'after-manager') { exit 97 }
    if (-not (Test-Hash $RouterPath $routerHash) -or -not (Test-Hash $ManagerPath $managerHash)) { Write-Fail '安装后二进制哈希验证失败；下次执行将先回滚。' }
    if ($env:MTLS_ROUTER_INSTALL_CRASH_POINT -eq 'before-receipt') { exit 97 }
    Commit-Pending
}

function Get-AssetUrl($Version, $Asset) {
    if ($DownloadBaseUrl) {
        if ($DownloadBaseUrl -match '/mtls-router-[^/]+$') { return (($DownloadBaseUrl.Substring(0, $DownloadBaseUrl.LastIndexOf('/'))) + "/$Asset") }
        return (($DownloadBaseUrl.TrimEnd('/')) + "/$Asset")
    }
    return "https://github.com/$Repo/releases/download/$Version/$Asset"
}

function Invoke-Download($Url, $OutFile) {
    if (([Uri]$Url).Scheme -ne 'https') { Write-Fail "拒绝非 HTTPS 下载地址：$Url" }
    $parameters = @{ Uri = $Url; OutFile = $OutFile; Headers = @{ 'User-Agent' = 'mtls-router-setup' } }
    if ($DownloadUser -or $DownloadPassword) {
        $secure = ConvertTo-SecureString -String ([string]$DownloadPassword) -AsPlainText -Force
        $parameters.Credential = [PSCredential]::new([string]$DownloadUser, $secure)
    }
    Invoke-WebRequest @parameters
}

function Install-MtlsRouterPair {
    if ($env:MTLS_ROUTER_SKIP_DOWNLOAD -eq '1') { Write-Info '[Download] skipped (MTLS_ROUTER_SKIP_DOWNLOAD=1)'; Repair-PendingInstall; return }
    $assets = Get-PlatformAssets
    $packagedRouter = Join-Path $ScriptDir $assets.Router
    $packagedManager = Join-Path $ScriptDir $assets.Manager
    $manifest = Join-Path $ScriptDir 'SHA256SUMS'
    $anyPayload = @(Get-ChildItem -LiteralPath $ScriptDir -File -Filter 'mtls-router-*-*' -ErrorAction SilentlyContinue).Count -gt 0
    if ($anyPayload) {
        if (-not (Test-Path $packagedRouter -PathType Leaf) -or -not (Test-Path $packagedManager -PathType Leaf) -or -not (Test-Path $manifest -PathType Leaf)) { Write-Fail '检测到不完整的随包二进制对；拒绝联网回退。' }
        Test-Checksum $packagedRouter $manifest $assets.Router
        Test-Checksum $packagedManager $manifest $assets.Manager
        Install-Pair $packagedRouter $packagedManager
        Write-Success "  已验证并安装 mtls-router 与 manager：$InstallDir"
        return
    }
    if (-not $AllowDownload) {
        $interactive = $false
        try { $interactive = -not [Console]::IsInputRedirected -and $null -ne $Host.UI.RawUI } catch {}
        if (-not $interactive) { Write-Fail '未找到随包二进制；非交互安装需显式传入 --download。' }
        if ((Read-Host '未找到随包二进制，是否通过 HTTPS 下载 router 与 manager？[y/N]') -notin @('y', 'Y')) { Write-Fail '已取消联网下载。' }
    }
    $version = if ($env:MTLS_ROUTER_VERSION) { $env:MTLS_ROUTER_VERSION } elseif ($DownloadBaseUrl) { 'latest' } else {
        try { (Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'mtls-router-setup' }).tag_name } catch { Write-Fail '无法获取 GitHub 最新 release。' }
    }
    $routerUrl = Get-AssetUrl $version $assets.Router
    $managerUrl = Get-AssetUrl $version $assets.Manager
    $manifestUrl = Get-AssetUrl $version 'SHA256SUMS'
    if (([Uri]$routerUrl).Scheme -ne 'https' -or ([Uri]$managerUrl).Scheme -ne 'https' -or ([Uri]$manifestUrl).Scheme -ne 'https') { Write-Fail '拒绝非 HTTPS 下载地址。' }
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('mtls-router-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $tmpRouter = Join-Path $tmp $assets.Router; $tmpManager = Join-Path $tmp $assets.Manager; $tmpManifest = Join-Path $tmp 'SHA256SUMS'
        Invoke-Download $routerUrl $tmpRouter
        Invoke-Download $managerUrl $tmpManager
        Invoke-Download $manifestUrl $tmpManifest
        Test-Checksum $tmpRouter $tmpManifest $assets.Router
        Test-Checksum $tmpManager $tmpManifest $assets.Manager
        Install-Pair $tmpRouter $tmpManager
    } finally { Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue }
    Write-Success "  已安装 mtls-router 与 manager：$InstallDir"
}

function Resolve-ManagerPair {
    Repair-PendingInstall
    $assets = Get-PlatformAssets
    $siblingRouter = Join-Path $ScriptDir $assets.Router
    $siblingManager = Join-Path $ScriptDir $assets.Manager
    $manifest = Join-Path $ScriptDir 'SHA256SUMS'
    $anyPayload = @(Get-ChildItem -LiteralPath $ScriptDir -File -Filter 'mtls-router-*-*' -ErrorAction SilentlyContinue).Count -gt 0
    if ($anyPayload) {
        if (-not (Test-Path $siblingRouter -PathType Leaf) -or -not (Test-Path $siblingManager -PathType Leaf) -or -not (Test-Path $manifest -PathType Leaf)) { Write-Fail '随包二进制对不完整；Agent 命令不会联网下载。' }
        Test-Checksum $siblingRouter $manifest $assets.Router
        Test-Checksum $siblingManager $manifest $assets.Manager
        return [pscustomobject]@{ Router = $siblingRouter; Manager = $siblingManager }
    }
    if (-not (Test-Receipt)) { Write-Fail '未找到 receipt 验证的 manager/router；Agent 命令不会隐式下载。请先运行 router install。' }
    return [pscustomobject]@{ Router = $RouterPath; Manager = $ManagerPath }
}

function Invoke-Manager($Request) {
    $pair = Resolve-ManagerPair
    return Invoke-ManagerAt $pair.Manager $pair.Router $Request
}

function Invoke-ManagerSecret($Request) {
    $pair = Resolve-ManagerPair
    $responses = @('{"id":"setup-secret-info","method":"manager.info","params":{}}' | & $pair.Manager serve --router-sidecar $pair.Router)
    if ($LASTEXITCODE -ne 0 -or $responses.Count -ne 1) { Write-Fail 'manager 返回了无效的握手响应。' }
    try { $info = $responses[0] | ConvertFrom-Json } catch { Write-Fail 'manager 返回了无效 JSON。' }
    $assets = Get-PlatformAssets
    $expectedTarget = 'windows/' + $(if ($assets.Manager -match '-arm64\.exe$') { 'arm64' } else { 'amd64' })
    if ($info.id -cne 'setup-secret-info' -or $info.error -or -not $info.result.deployment_id -or [string]$info.result.target -cne $expectedTarget -or [string]$info.result.management_protocol_version -cne $ManagementProtocolVersion) {
        Write-Fail 'manager protocol v2 握手失败；未接受含密钥请求。'
    }
    if ($pair.Manager -ceq $ManagerPath) {
        $receipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
        if ([string]$info.result.deployment_id -cne [string]$receipt.deployment_id) { Write-Fail 'manager deployment 握手与安装 receipt 不匹配；未接受含密钥请求。' }
    }
    $responses = @($Request | & $pair.Manager serve --router-sidecar $pair.Router)
    if ($LASTEXITCODE -ne 0 -or $responses.Count -ne 1) { Write-Fail 'manager 返回了无效的响应。' }
    try { $operation = $responses[0] | ConvertFrom-Json } catch { Write-Fail 'manager 返回了无效 JSON。' }
    if ($operation.error) { Write-Fail "manager $($operation.error.code): $($operation.error.message)" }
    return $operation
}

function Start-MtlsRouter {
    if ($env:MTLS_ROUTER_SKIP_START -eq '1') { Write-Info '[启动] 跳过（MTLS_ROUTER_SKIP_START=1）'; return }
    $result = Invoke-Manager '{"id":"setup","method":"router.start","params":{"owner":"cli"}}'
    Write-Success "  mtls-router 已启动，监听地址通常为 $RouterBaseUrl"
    Write-Info "pid=$($result.result.pid)"
}

function Show-RouterStatus {
    $response = Invoke-Manager '{"id":"status","method":"router.status","params":{}}'
    switch ($response.result.state) {
        { $_ -in @('external_compatible', 'desktop_owned', 'degraded') } { Write-Success 'router running' }
        { $_ -in @('stale', 'unknown_occupant') } { Write-Warn 'router state stale' }
        'absent' { Write-Info 'router 未运行' }
        default { Write-Info "router $($response.result.state)" }
    }
    Write-Info "pid=$($response.result.pid)"
    Write-Info "listen_addr=$($response.result.listen_addr)"
    $state = Join-Path $StateDir 'setup-state.json'
    if (Test-Path $state) {
        $saved = Get-Content $state -Raw | ConvertFrom-Json
        Write-Info "binary_path=$($saved.binary_path)"
        Write-Info "log_path=$($saved.log_path)"
    }
}

function Show-RouterLog($Lines) {
    if ([int]$Lines -lt 1 -or [int]$Lines -gt 1000) { Write-Fail 'router log --tail 需要 1-1000 的行数。' }
    $request = [ordered]@{ id = 'logs'; method = 'router.logs'; params = @{ limit = [int]$Lines } } | ConvertTo-Json -Compress -Depth 5
    (Invoke-Manager $request).result.lines | ForEach-Object { Write-Host $_ }
}

function Stop-Router {
    $pair = Resolve-ManagerPair
    $response = '{"id":"stop","method":"router.stop","params":{}}' | & $pair.Manager serve --router-sidecar $pair.Router
    if ($LASTEXITCODE -ne 0) { Write-Fail 'mtls-router-manager 执行失败。' }
    try { $decoded = @($response)[0] | ConvertFrom-Json } catch { Write-Fail 'manager 返回了无效 JSON。' }
    if ($decoded.error.code -eq 'ROUTER_NOT_FOUND') { Write-Info 'router 未运行'; return }
    if ($decoded.error) { Write-Fail "manager $($decoded.error.code): $($decoded.error.message)" }
    Write-Success 'router stopped'
}

function Get-AgentTargets($Filter, $Detect) {
    if (-not $Filter) { return @($Detect.result.agents | Where-Object detected | ForEach-Object agent) }
    $seen = @{}; $targets = @()
    foreach ($token in @($Filter -split ',' | ForEach-Object Trim | Where-Object { $_ })) {
        if ($token -notin @('claude', 'opencode', 'codex')) { Write-Fail "未知 agent: $token" }
        if ($seen[$token]) { Write-Fail "--agent 列表中有重复项：$token" }
        if (-not @($Detect.result.agents | Where-Object { $_.agent -eq $token -and $_.detected }).Count) { Write-Fail "未检测到 agent: $token" }
        $seen[$token] = $true; $targets += $token
    }
    return $targets
}

function Get-PropertyValue($Object, [string]$Name) { if ($null -ne $Object -and $Object.PSObject.Properties[$Name]) { return $Object.PSObject.Properties[$Name].Value }; return $null }
function Read-StringDefault([string]$Label, [AllowEmptyString()][string]$Default = '') { $suffix = if ($Default) { " [$Default]" } else { '' }; $value = Read-Host "$Label$suffix"; if ([string]::IsNullOrEmpty($value)) { return $Default }; return $value }
function Read-OptionalString($Label, $Default = $null) { $suffix = if ($null -ne $Default -and [string]$Default) { " [默认: $Default；留空接受，输入 - 清除]" } else { ' [留空表示未设置]' }; $v = Read-Host "$Label$suffix"; if ([string]::IsNullOrEmpty($v)) { return $Default }; if ($v -ceq '-') { return $null }; return $v }
function Read-OptionalBoolean($Label, $Default = $null) { while ($true) { $shown = if ($null -eq $Default) { $null } else { ([string]$Default).ToLowerInvariant() }; $v = Read-OptionalString "$Label true/false" $shown; if ($null -eq $v) { return $null }; if ($v -ceq 'true') { return $true }; if ($v -ceq 'false') { return $false } } }
function Read-OptionalInteger($Label, $Default = $null) { $v = Read-OptionalString "$Label 正整数" $(if ($null -eq $Default) { $null } else { [string]$Default }); if ($null -eq $v) { return $null }; $n = 0L; if (-not [Int64]::TryParse($v, [ref]$n) -or $n -le 0) { Write-Fail "$Label 必须是正整数。" }; return $n }
function Read-OptionalObject($Label, $Default = $null) { $defaultJson = if ($null -eq $Default) { $null } else { $Default | ConvertTo-Json -Compress -Depth 20 }; $v = Read-OptionalString "$Label JSON object" $defaultJson; if ($null -eq $v) { return $null }; try { $o = $v | ConvertFrom-Json } catch { Write-Fail "$Label 必须是 JSON object。" }; if ($o -isnot [pscustomobject]) { Write-Fail "$Label 必须是 JSON object。" }; return $o }
function Read-ClaudeContext($Label, $Default = $null) { $shown = if ([string]$Default -ceq '1m') { '1m' } else { 'standard' }; $value = Read-StringDefault "$Label context（standard/1m）" $shown; if ($value -cnotin @('standard', '1m')) { Write-Fail "$Label context 只接受 standard 或 1m。" }; return $value }
function Add-Optional($Object, $Name, $Value) { if ($null -ne $Value) { $Object[$Name] = $Value } }

function Get-AgentInitialConfig($Targets, $Models) {
    $config = [ordered]@{ version = 1 }
    foreach ($agent in $Targets) {
        $existing = Get-PropertyValue $Models.result.existing.model_config $agent
        $preset = Get-PropertyValue $Models.result.preset.model_config $agent
        if ($null -ne $existing) { $config[$agent] = $existing; Write-Info "CONFIG SOURCE ${agent}: existing" }
        elseif ($null -ne $preset) { $config[$agent] = $preset; Write-Info "CONFIG SOURCE ${agent}: preset" }
        else { Write-Info "CONFIG SOURCE ${agent}: empty" }
        $unavailable = Get-PropertyValue $Models.result.preset.unavailable_agents $agent
        if ($null -ne $unavailable -and @($unavailable.models).Count) { Write-Warn "PRESET UNAVAILABLE ${agent}: $(@($unavailable.models) -join ', ')" }
    }
    return [pscustomobject]$config
}

function New-AgentModelConfig($Targets, $InitialConfig) {
    $config = [ordered]@{ version = 1 }
    foreach ($agent in $Targets) {
        $initial = Get-PropertyValue $InitialConfig $agent
        switch ($agent) {
            'claude' {
                $primaryDefault = Get-PropertyValue $initial 'primary'
                $model = Read-StringDefault 'Claude primary model ID（不会自动选择）' ([string](Get-PropertyValue $primaryDefault 'model')); if (-not $model) { Write-Fail 'model ID 不能为空。' }
                $primary = [ordered]@{ model = $model }; Add-Optional $primary 'name' (Read-OptionalString 'Claude primary name' (Get-PropertyValue $primaryDefault 'name')); $context = Read-ClaudeContext 'Claude primary' (Get-PropertyValue $primaryDefault 'context'); if ($context -ceq '1m') { $primary.context = '1m' }
                $section = [ordered]@{ primary = $primary }
                foreach ($role in @('haiku','sonnet','opus')) { $roleDefault = Get-PropertyValue $initial $role; $choiceDefault = if ((Get-PropertyValue $roleDefault 'inherit_primary') -eq $true) { 'inherit' } else { [string](Get-PropertyValue $roleDefault 'model') }; $choice = Read-StringDefault "Claude ${role}：输入 inherit 或 model ID" $choiceDefault; if (-not $choice) { Write-Fail "$role 不能为空。" }; if ($choice -ceq 'inherit') { $section[$role] = [ordered]@{ inherit_primary = $true } } else { $entry = [ordered]@{ model = $choice }; Add-Optional $entry 'name' (Read-OptionalString "Claude $role name" (Get-PropertyValue $roleDefault 'name')); $context = Read-ClaudeContext "Claude $role" (Get-PropertyValue $roleDefault 'context'); if ($context -ceq '1m') { $entry.context = '1m' }; $section[$role] = $entry } }
                $initialExtra = Get-PropertyValue $initial 'extra'; $extra = [ordered]@{}; foreach ($field in @('ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION','ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION','ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION','ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION')) { Add-Optional $extra $field (Read-OptionalString "Claude $field" (Get-PropertyValue $initialExtra $field)) }; if ($extra.Count) { $section.extra = $extra }; $config.claude = $section
            }
            'opencode' {
                $initialModels = Get-PropertyValue $initial 'models'; $idDefault = if ($null -eq $initialModels) { '' } else { @($initialModels.PSObject.Properties.Name) -join ',' }; $ids = @((Read-StringDefault 'opencode model IDs（逗号分隔，不会自动选择）' $idDefault).Split(',') | ForEach-Object Trim | Where-Object { $_ }); if (-not $ids.Count) { Write-Fail '至少需要一个 model。' }; $default = Read-StringDefault 'opencode default model ID' ([string](Get-PropertyValue $initial 'default_model')); if (-not $default) { Write-Fail 'opencode default model ID 不能为空。' }; $models = [ordered]@{}
                foreach ($model in $ids) { $modelDefault = Get-PropertyValue $initialModels $model; $entry = [ordered]@{}; Add-Optional $entry 'name' (Read-OptionalString "opencode $model name" (Get-PropertyValue $modelDefault 'name')); foreach ($field in @('reasoning','attachment','tool_call','temperature')) { Add-Optional $entry $field (Read-OptionalBoolean "opencode $model $field" (Get-PropertyValue $modelDefault $field)) }; $limitDefault = Get-PropertyValue $modelDefault 'limit'; $context = Read-OptionalInteger "opencode $model limit.context" (Get-PropertyValue $limitDefault 'context'); $input = Read-OptionalInteger "opencode $model limit.input" (Get-PropertyValue $limitDefault 'input'); $output = Read-OptionalInteger "opencode $model limit.output" (Get-PropertyValue $limitDefault 'output'); if ($null -ne $context -or $null -ne $input -or $null -ne $output) { if ($null -eq $context -or $null -eq $output) { Write-Fail 'limit 需要 context 和 output。' }; $limit = [ordered]@{ context = $context; output = $output }; Add-Optional $limit 'input' $input; $entry.limit = $limit }; Add-Optional $entry 'modalities' (Read-OptionalObject "opencode $model modalities（input/output arrays）" (Get-PropertyValue $modelDefault 'modalities')); $interleavedDefault = Get-PropertyValue $modelDefault 'interleaved'; if ($interleavedDefault -eq $true) { $interleavedDefault = 'true' } elseif ($null -ne $interleavedDefault) { $interleavedDefault = Get-PropertyValue $interleavedDefault 'field' }; $interleaved = Read-OptionalString "opencode $model interleaved: true/reasoning/reasoning_content/reasoning_details" $interleavedDefault; if ($interleaved -ceq 'true') { $entry.interleaved = $true } elseif ($interleaved) { $entry.interleaved = [ordered]@{ field = $interleaved } }; Add-Optional $entry 'options' (Read-OptionalObject "opencode $model options" (Get-PropertyValue $modelDefault 'options')); Add-Optional $entry 'extra' (Read-OptionalObject "opencode $model extra" (Get-PropertyValue $modelDefault 'extra')); $models[$model] = $entry }
                $config.opencode = [ordered]@{ default_model = $default; models = $models }
            }
            'codex' {
                $model = Read-StringDefault 'Codex model ID（不会自动选择）' ([string](Get-PropertyValue $initial 'model')); if (-not $model) { Write-Fail 'model ID 不能为空。' }; $section = [ordered]@{ model = $model }; foreach ($field in @('reasoning_effort','reasoning_summary','verbosity')) { Add-Optional $section $field (Read-OptionalString "Codex $field" (Get-PropertyValue $initial $field)) }; foreach ($field in @('context_window','auto_compact_token_limit')) { Add-Optional $section $field (Read-OptionalInteger "Codex $field" (Get-PropertyValue $initial $field)) }; $scope = Read-OptionalString 'Codex model_auto_compact_token_limit_scope: total/body_after_prefix' (Get-PropertyValue (Get-PropertyValue $initial 'extra') 'model_auto_compact_token_limit_scope'); if ($scope) { $section.extra = [ordered]@{ model_auto_compact_token_limit_scope = $scope } }; $config.codex = $section
            }
        }
    }
    return $config
}

function Read-ModelConfigFile($Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { Write-Fail '--model-config 必须是普通文件。' }; $item = Get-Item -LiteralPath $Path -Force; if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { Write-Fail '--model-config 不接受链接或 reparse point。' }; if ($item.Length -le 0 -or $item.Length -gt $MaxModelConfigSize) { Write-Fail '--model-config 必须大于 0 且不超过 2 MiB。' }; try { $config = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json } catch { Write-Fail '--model-config 必须是有效 JSON object。' }; if ($config -isnot [pscustomobject]) { Write-Fail '--model-config 根必须是 JSON object。' }; return $config
}
function Show-Fragments($Result) { foreach ($f in $Result.fragments) { Write-Host "### $($f.agent) [$($f.role)] -> $($f.path) ($($f.format))"; Write-Host $f.content; Write-Host '' } }
function Show-ExactPreview($Result) { Show-Fragments $Result; foreach ($f in $Result.files) { Write-Host "FILE $($f.operation): $($f.path)"; if ($f.backup_path) { Write-Host "  BACKUP: $($f.backup_path)" } }; foreach ($c in $Result.managed_collisions) { Write-Host "COLLISION $($c.agent) $($c.path): $($c.type) -> $($c.action)" }; if ($Result.state_change) { Write-Host "STATE $($Result.state_change.operation): $($Result.state_change.path)" }; if ($Result.state_backup) { Write-Host "STATE BACKUP: $($Result.state_backup.path)" } }

function Invoke-AgentFlow($Mode, $Filter, $ModelConfigPath) {
    if ($Mode -ceq 'write' -and -not $Filter) { Write-Fail '--write-config 需要 --agent=claude,opencode,codex 显式指定要操作的 agent。' }; $detect = Invoke-Manager '{"id":"detect","method":"agent.detect","params":{}}'; if (-not $Filter) { if ([Console]::IsInputRedirected) { Write-Fail 'Agent 配置需要交互选择。非交互自动化请使用 manager protocol v2 stdin。' }; foreach ($state in @($detect.result.agents | Where-Object detected)) { Write-Host "DETECTED $($state.agent): $($state.path)" }; $Filter = Read-Host '选择 Agent（逗号分隔 claude,opencode,codex）'; if (-not $Filter) { Write-Fail '必须显式选择至少一个 Agent。' } }; $targets = @(Get-AgentTargets $Filter $detect); if (-not $targets.Count) { Write-Warn '未检测到 Agent。'; return }
    if ([Console]::IsInputRedirected) { Write-Fail 'Agent 配置需要隐藏交互输入。非交互自动化请使用 manager protocol v2 stdin：manager.info 握手后调用 agent.models、agent.render/agent.preview、agent.write；key 不得进入参数、环境或文件。MTLS_ROUTER_OPENAI_API_KEY 已移除。' }
    try { $secure = Read-Host '请输入 mtls-router OPENAI_API_KEY（输入隐藏）' -AsSecureString; $ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure); try { $script:TransientApiKey = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr) } finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr) }; if (-not $script:TransientApiKey) { Write-Fail 'API key 不能为空。' }
        $script:TransientRequest = [ordered]@{ id='models'; method='agent.models'; params=[ordered]@{ owner='cli'; agents=$targets; api_key=$script:TransientApiKey } } | ConvertTo-Json -Compress -Depth 30; $models = Invoke-ManagerSecret $script:TransientRequest; $script:TransientRequest = ''; foreach ($id in $models.result.models) { Write-Host "MODEL $id" }
        $modelConfig = if ($ModelConfigPath) { Read-ModelConfigFile $ModelConfigPath } else { $initialConfig = Get-AgentInitialConfig $targets $models; New-AgentModelConfig $targets $initialConfig }; $params = [ordered]@{ agents=$targets; catalog_token=$models.result.catalog_token; model_config=$modelConfig }
        if ($Mode -ceq 'print') { $render = Invoke-Manager ([ordered]@{ id='render'; method='agent.render'; params=$params } | ConvertTo-Json -Compress -Depth 30); Show-Fragments $render.result; Write-Info 'manager 动态渲染完成；未修改 Agent、事务或 sidecar 文件。发现模型可能启动 router 生命周期状态。'; return }
        $preview = Invoke-Manager ([ordered]@{ id='preview'; method='agent.preview'; params=$params } | ConvertTo-Json -Compress -Depth 30); Show-ExactPreview $preview.result; $approveDrift = $false; $approveCodex = $false; if ($preview.result.managed_config_drift) { if ((Read-Host 'managed drift：输入 OVERWRITE 批准') -cne 'OVERWRITE') { Write-Fail '已取消。' }; $approveDrift = $true }; if ($preview.result.requires_codex_auth_approval) { if ((Read-Host 'Codex auth 将变更并清理冲突认证；输入 CODEX-AUTH 批准') -cne 'CODEX-AUTH') { Write-Fail '已取消。' }; $approveCodex = $true }; if ((Read-Host '输入 WRITE 确认写入') -cne 'WRITE') { Write-Fail '已取消。' }
        $writeParams = [ordered]@{ agents=$targets; catalog_token=$models.result.catalog_token; model_config=$modelConfig; revision_token=$preview.result.revision_token; approve_managed_overwrite=$approveDrift; approve_codex_auth_change=$approveCodex; api_key=$script:TransientApiKey }; $script:TransientRequest = [ordered]@{ id='write'; method='agent.write'; params=$writeParams } | ConvertTo-Json -Compress -Depth 30; $response = Invoke-ManagerSecret $script:TransientRequest; foreach ($agent in $response.result.agents) { foreach ($path in $agent.changed) { Write-Success "WROTE $($agent.agent): $path" }; foreach ($backup in $agent.backups) { Write-Info "BACKUP: $backup" } }
    } finally { $script:TransientApiKey = ''; $script:TransientRequest = ''; $writeParams = $null; $secure = $null; $modelConfig = $null; $initialConfig = $null }
}

function Print-NextSteps {
    Write-Success '============================================================'
    Write-Success '配置完成。'
    Write-Success '============================================================'
    if ($script:RouterStarted) { Write-Info "mtls-router 已在后台运行：`n  $RouterBaseUrl" }
    else { Write-Info "未启动 mtls-router（本次仅处理配置）。如需启动，请运行：`n  $PSCommandPath router start" }
}

function Show-Usage {
@'
用法: setup.ps1 [router|agent] <command> [选项]

默认行为等价于 router setup，不修改 Agent 配置。

  router install|start|setup|status|log [--tail=N]|stop
  agent print-config [--agent=claude,opencode,codex] [--model-config=PATH]
  agent write-config --agent=claude,opencode,codex [--model-config=PATH]
  --print-config / --write-config   兼容旧别名

安装选项: --download --download-url=URL --download-user=USER
          --download-password=PASSWORD --version=VERSION

Agent 命令先隐藏读取 key，再发现模型；不会自动选择模型。向导覆盖 Agent-native
typed fields，或读取不超过 2 MiB 的普通 JSON --model-config 文件。
非交互自动化请使用 manager protocol v2 stdin：manager.info 握手后依次调用
agent.models、agent.render/agent.preview、agent.write；key 不得进入参数、环境或文件。
'@
}

function Main {
    $action = 'setup'; $agentFilter = ''; $modelConfigPath = ''; $lines = 200
    if ($args.Count) {
        switch ($args[0]) {
            'router' { if ($args.Count -lt 2) { Write-Fail 'router 需要子命令' }; $action = "router-$($args[1])"; $args = @($args | Select-Object -Skip 2) }
            'agent' { if ($args.Count -lt 2) { Write-Fail 'agent 需要子命令' }; $action = switch ($args[1]) { 'print-config' {'print'} 'write-config' {'write'} default { Write-Fail "未知 agent 子命令：$($args[1])" } }; $args = @($args | Select-Object -Skip 2) }
            '--print-config' { $action = 'print'; $args = @($args | Select-Object -Skip 1) }
            '--write-config' { $action = 'write'; $args = @($args | Select-Object -Skip 1) }
            { $_ -in @('-h', '--help') } { Show-Usage; return }
            default { if ($args[0] -notlike '--*') { Write-Fail "未知参数：$($args[0])" } }
        }
    }
    if ($action -notin @('setup','router-install','router-start','router-setup','router-status','router-log','router-stop','print','write')) { Write-Fail "未知 router 子命令：$($action.Substring(7))" }
    $i = 0
    while ($i -lt $args.Count) {
        $a = $args[$i]
        switch -Regex ($a) {
            '^--agent=(.+)$' { $agentFilter = $Matches[1]; $i++; continue }
            '^--agent$' { if (++$i -ge $args.Count) { Write-Fail '--agent 需要参数' }; $agentFilter = $args[$i]; $i++; continue }
            '^--model-config=(.+)$' { $modelConfigPath = $Matches[1]; $i++; continue }
            '^--model-config$' { if (++$i -ge $args.Count) { Write-Fail '--model-config 需要路径' }; $modelConfigPath = $args[$i]; $i++; continue }
            '^--tail=(.+)$' { $lines = $Matches[1]; $i++; continue }
            '^--tail$' { if (++$i -ge $args.Count) { Write-Fail '--tail 需要行数' }; $lines = $args[$i]; $i++; continue }
            '^--download$' { if ($action -notin @('setup','router-install','router-setup')) { Write-Fail '--download 仅适用于 router install/setup' }; $script:AllowDownload = $true; $i++; continue }
            '^--download-url=(.+)$' { $script:DownloadBaseUrl = $Matches[1]; $i++; continue }
            '^--download-url$' { $script:DownloadBaseUrl = $args[++$i]; $i++; continue }
            '^--download-user=(.*)$' { $script:DownloadUser = $Matches[1]; $i++; continue }
            '^--download-user$' { $script:DownloadUser = $args[++$i]; $i++; continue }
            '^--download-password=(.*)$' { $script:DownloadPassword = $Matches[1]; $i++; continue }
            '^--download-password$' { $script:DownloadPassword = $args[++$i]; $i++; continue }
            '^--version=(.+)$' { $env:MTLS_ROUTER_VERSION = $Matches[1]; $i++; continue }
            '^--version$' { $env:MTLS_ROUTER_VERSION = $args[++$i]; $i++; continue }
            '^(-h|--help)$' { Show-Usage; return }
            default { Write-Fail "未知参数：$a" }
        }
    }
    Show-Banner
    switch ($action) {
        { $_ -in @('setup','router-setup') } { Install-MtlsRouterPair; Start-MtlsRouter; if ($env:MTLS_ROUTER_SKIP_START -ne '1') { $script:RouterStarted = $true }; Write-Info '提示：未对 agent 配置做任何改动。' }
        'router-install' { Install-MtlsRouterPair; Print-NextSteps }
        'router-start' { Start-MtlsRouter; if ($env:MTLS_ROUTER_SKIP_START -ne '1') { $script:RouterStarted = $true }; Print-NextSteps }
        'router-status' { Show-RouterStatus }
        'router-log' { Show-RouterLog $lines }
        'router-stop' { Stop-Router }
        'print' { Invoke-AgentFlow 'print' $agentFilter $modelConfigPath }
        'write' { Invoke-AgentFlow 'write' $agentFilter $modelConfigPath; Print-NextSteps }
    }
}

Main @args
