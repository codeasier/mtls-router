$ErrorActionPreference = 'Stop'

$Repo = 'codeasier/mtls-router'
$RouterBaseUrl = 'http://127.0.0.1:19099'
$InstallDir = if ($env:MTLS_ROUTER_INSTALL_DIR) { $env:MTLS_ROUTER_INSTALL_DIR } else { Join-Path $env:USERPROFILE '.local\bin' }
$DefaultDownloadBaseUrl = ''
$DownloadBaseUrl = if ($env:MTLS_ROUTER_DOWNLOAD_URL) { $env:MTLS_ROUTER_DOWNLOAD_URL } else { $DefaultDownloadBaseUrl }
$DownloadUser = $env:MTLS_ROUTER_DOWNLOAD_USER
$DownloadPassword = $env:MTLS_ROUTER_DOWNLOAD_PASSWORD
$BinaryPath = Join-Path $InstallDir 'mtls-router.exe'
$RouterStateDir = if ($env:MTLS_ROUTER_STATE_DIR) { $env:MTLS_ROUTER_STATE_DIR } else { Join-Path $env:USERPROFILE '.mtls-router' }
$RouterStatePath = Join-Path $RouterStateDir 'setup-state.json'
$RouterLogPath = if ($env:MTLS_ROUTER_LOG_PATH) { $env:MTLS_ROUTER_LOG_PATH } else { Join-Path $RouterStateDir 'mtls-router.log' }
$ConfiguredAgentPaths = @()
$ConfiguredBackups = @()
$DetectedAgents = @()

function Write-Info($Message) { Write-Host $Message -ForegroundColor Cyan }
function Write-Success($Message) { Write-Host $Message -ForegroundColor Green }
function Write-Warn($Message) { Write-Host $Message -ForegroundColor Yellow }
function Write-Fail($Message) { Write-Host $Message -ForegroundColor Red; exit 1 }

function Show-Banner {
    Write-Info '============================================================'
    Write-Info ' mtls-router 代理配置向导'
    Write-Info '============================================================'
}

function Get-RouterAssetName {
    $arch = if ([Environment]::Is64BitOperatingSystem -and $env:PROCESSOR_ARCHITECTURE -match 'ARM64') {
        'arm64'
    } elseif ($env:PROCESSOR_ARCHITEW6432 -match 'ARM64') {
        'arm64'
    } elseif ($env:PROCESSOR_ARCHITECTURE -match 'AMD64|x86_64') {
        'amd64'
    } else {
        Write-Fail "不支持的 CPU 架构：$env:PROCESSOR_ARCHITECTURE"
    }

    "mtls-router-windows-$arch.exe"
}

function Get-DownloadUrl($Version, $Asset) {
    if ($DownloadBaseUrl) {
        if ($DownloadBaseUrl.EndsWith("/$Asset")) {
            return $DownloadBaseUrl
        }
        return (($DownloadBaseUrl.TrimEnd('/')) + "/$Asset")
    }

    return "https://github.com/$Repo/releases/download/$Version/$Asset"
}

function Invoke-Download($Url, $OutFile) {
    $headers = @{ 'User-Agent' = 'mtls-router-setup' }
    $parameters = @{ Uri = $Url; OutFile = $OutFile; Headers = $headers }
    if ($DownloadUser -or $DownloadPassword) {
        $securePassword = ConvertTo-SecureString -String ([string]$DownloadPassword) -AsPlainText -Force
        $parameters['Credential'] = [PSCredential]::new([string]$DownloadUser, $securePassword)
    }
    Invoke-WebRequest @parameters
}

function Download-MtlsRouter {
    Write-Info '[下载] 检测并下载最新 mtls-router...'
    $asset = Get-RouterAssetName

    if ($DownloadBaseUrl) {
        $version = if ($env:MTLS_ROUTER_VERSION) { $env:MTLS_ROUTER_VERSION } else { 'latest' }
    } else {
        try {
            $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'mtls-router-setup' }
        } catch {
            Write-Fail "无法获取 GitHub 最新 release：$($_.Exception.Message)"
        }

        $version = $release.tag_name
        if (-not $version) { Write-Fail '无法读取最新 release 版本。' }
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $url = Get-DownloadUrl $version $asset

    Write-Info "  版本：$version"
    Write-Info "  平台：$asset"
    Write-Info "  安装：$BinaryPath"

    try {
        Invoke-Download $url $BinaryPath
    } catch {
        Write-Fail "下载 mtls-router 失败：$($_.Exception.Message)"
    }

    Write-Success "  已安装 mtls-router：$BinaryPath"
}

function Write-RouterState($PidValue, $LogPath) {
    New-Item -ItemType Directory -Force -Path $RouterStateDir | Out-Null
    $tmp = [System.IO.Path]::GetTempFileName()
    [ordered]@{
        pid = [int]$PidValue
        listen_addr = $RouterBaseUrl
        binary_path = $BinaryPath
        log_path = $LogPath
        started_at = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    } | ConvertTo-Json -Depth 5 | Set-Content -Path $tmp -Encoding UTF8
    Move-Item -Path $tmp -Destination $RouterStatePath -Force
}

function Get-RouterState {
    if (-not (Test-Path $RouterStatePath)) { return $null }
    try {
        return Get-Content $RouterStatePath -Raw | ConvertFrom-Json
    } catch {
        Write-Fail "router 状态文件不是合法 JSON：$RouterStatePath"
    }
}

function Test-RouterProcess($PidValue) {
    if (-not $PidValue) { return $false }
    return $null -ne (Get-Process -Id $PidValue -ErrorAction SilentlyContinue)
}

function Show-RouterStatus {
    $state = Get-RouterState
    if (-not $state) {
        Write-Info "未找到 setup-managed router 状态文件：$RouterStatePath"
        Write-Info 'router 未运行'
        return
    }
    if (Test-RouterProcess $state.pid) {
        Write-Success 'router running'
    } else {
        Write-Warn 'router 未运行'
    }
    Write-Info "pid=$($state.pid)"
    Write-Info "listen_addr=$($state.listen_addr)"
    Write-Info "binary_path=$($state.binary_path)"
    Write-Info "log_path=$($state.log_path)"
    if ($state.started_at) { Write-Info "started_at=$($state.started_at)" }
}

function Show-RouterLog($Lines = 200) {
    if ($Lines -notmatch '^[0-9]+$') { Write-Fail 'router log --tail 需要正整数行数。' }
    $state = Get-RouterState
    if (-not $state) { Write-Fail "未找到 setup-managed router 状态文件：$RouterStatePath" }
    if (-not $state.log_path) { Write-Fail "状态文件中没有 log_path：$RouterStatePath" }
    if (-not (Test-Path $state.log_path)) { Write-Fail "router 日志文件不存在：$($state.log_path)" }
    Get-Content -Path $state.log_path -Tail ([int]$Lines)
}

function Stop-Router {
    $state = Get-RouterState
    if (-not $state) {
        Write-Info "未找到 setup-managed router 状态文件：$RouterStatePath"
        Write-Info 'router 未运行'
        return
    }
    if (Test-RouterProcess $state.pid) {
        Stop-Process -Id $state.pid -ErrorAction SilentlyContinue
        $deadline = (Get-Date).AddSeconds(5)
        while ((Test-RouterProcess $state.pid) -and ((Get-Date) -lt $deadline)) {
            Start-Sleep -Milliseconds 200
        }
        if (Test-RouterProcess $state.pid) {
            Write-Warn 'router 未正常退出，强制停止。'
            Stop-Process -Id $state.pid -Force -ErrorAction SilentlyContinue
        }
        Write-Success 'router stopped'
    } else {
        Write-Warn 'router 未运行'
    }
    Remove-Item -Path $RouterStatePath -Force -ErrorAction SilentlyContinue
}

function Start-MtlsRouter {
    if ($env:MTLS_ROUTER_SKIP_START -eq '1') {
        Write-Info '[启动] 跳过（MTLS_ROUTER_SKIP_START=1）'
        return
    }
    if (-not (Test-Path $BinaryPath)) {
        Write-Fail "未找到已安装的 mtls-router：$BinaryPath。请先运行 router install 或 router setup。"
    }
    Write-Info '[启动] 启动 mtls-router 后台模式...'
    New-Item -ItemType Directory -Force -Path $RouterStateDir | Out-Null
    $output = & $BinaryPath -backend -log $RouterLogPath 2>&1
    $exitCode = $LASTEXITCODE
    $output | ForEach-Object { Write-Host $_ }
    if ($exitCode -ne 0) { Write-Fail 'mtls-router 后台启动失败。' }
    $text = ($output | Out-String)
    $pidMatch = [regex]::Match($text, '(?m)^mtls-router started in background, pid=([0-9]+), log=(.*)$')
    if (-not $pidMatch.Success) { Write-Fail '无法从 mtls-router 输出中读取后台 pid。' }
    $logPath = $pidMatch.Groups[2].Value.Trim()
    if (-not $logPath) { $logPath = $RouterLogPath }
    Write-RouterState $pidMatch.Groups[1].Value $logPath
    Write-Success "  mtls-router 已启动，监听地址通常为 $RouterBaseUrl"
}

function Refresh-Path {
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = "$machinePath;$userPath"
}

function Set-ObjectProperty($Object, $Name, $Value) {
    $property = $Object.PSObject.Properties[$Name]
    if ($property) {
        $property.Value = $Value
    } else {
        $Object | Add-Member -MemberType NoteProperty -Name $Name -Value $Value
    }
}

function ConvertTo-Hashtable($Value) {
    if ($Value -is [System.Collections.IDictionary]) {
        $result = [ordered]@{}
        foreach ($key in $Value.Keys) { $result[$key] = ConvertTo-Hashtable $Value[$key] }
        return $result
    }
    if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [string]) {
        return @($Value | ForEach-Object { ConvertTo-Hashtable $_ })
    }
    if ($null -ne $Value -and $Value.PSObject.TypeNames -contains 'System.Management.Automation.PSCustomObject') {
        $result = [ordered]@{}
        foreach ($property in $Value.PSObject.Properties) { $result[$property.Name] = ConvertTo-Hashtable $property.Value }
        return $result
    }
    return $Value
}

function Read-JsonObject($Path) {
    ConvertTo-Hashtable (Get-Content $Path -Raw | ConvertFrom-Json)
}

function Detect-Agents {
    $script:DetectedAgents = @()

    $claude = Get-Command claude -ErrorAction SilentlyContinue
    if ($claude -and $claude.Source) {
        $configPath = if ($env:CLAUDE_CONFIG_DIR) {
            Join-Path $env:CLAUDE_CONFIG_DIR 'settings.json'
        } else {
            Join-Path (Join-Path $env:USERPROFILE '.claude') 'settings.json'
        }
        $script:DetectedAgents += [PSCustomObject]@{
            Name = 'Claude Code'
            Command = $claude.Source
            ConfigPath = $configPath
        }
    }

    $opencode = Get-Command opencode -ErrorAction SilentlyContinue
    if ($opencode -and $opencode.Source) {
        if ($env:OPENCODE_CONFIG) {
            $configPath = $env:OPENCODE_CONFIG
        } else {
            $jsonPath = Join-Path (Join-Path $env:USERPROFILE '.config\opencode') 'opencode.json'
            $jsoncPath = Join-Path (Join-Path $env:USERPROFILE '.config\opencode') 'opencode.jsonc'
            if (Test-Path $jsonPath) {
                $configPath = $jsonPath
            } elseif (Test-Path $jsoncPath) {
                $configPath = $jsoncPath
            } else {
                $configPath = $jsonPath
            }
        }
        $script:DetectedAgents += [PSCustomObject]@{
            Name = 'opencode'
            Command = $opencode.Source
            ConfigPath = $configPath
        }
    }

    $codex = Get-Command codex -ErrorAction SilentlyContinue
    $codexHome = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $env:USERPROFILE '.codex' }
    # Detect Codex when either the CLI is on PATH, or the Codex Desktop
    # app has been used (which writes ~/.codex/config.toml and auth.json
    # without ever installing the CLI).
    if (($codex -and $codex.Source) -or (Test-Path -PathType Container $codexHome)) {
        $script:DetectedAgents += [PSCustomObject]@{
            Name = 'Codex'
            Command = if ($codex -and $codex.Source) { $codex.Source } else { '<desktop>' }
            ConfigPath = Join-Path $codexHome 'config.toml'
            AuthPath = Join-Path $codexHome 'auth.json'
        }
    }
}

function Select-Targets($Selection, $Total) {
    if ($Total -le 0) { return @() }
    if ([string]::IsNullOrWhiteSpace($Selection)) {
        return @(1..$Total)
    }

    $tokens = @($Selection -split '\s+' | Where-Object { $_ })
    if ($tokens -contains '0') {
        return @(1..$Total)
    }

    $result = New-Object System.Collections.Generic.List[int]
    foreach ($token in $tokens) {
        if ($token -notmatch '^[0-9]+$') {
            Write-Fail "无效编号：$token"
        }
        if ($token -match '^0[0-9]+$') {
            Write-Fail "无效编号：$token"
        }
        $num = [int]$token
        if ($num -lt 1 -or $num -gt $Total) {
            Write-Fail "编号越界：$token（有效范围 1-$Total）"
        }
        if (-not $result.Contains($num)) {
            $result.Add($num)
        }
    }
    return @($result)
}

function Backup-File($Path) {
    if (Test-Path $Path) {
        $stamp = '{0:yyyyMMdd-HHmmssfffffff}-{1}' -f (Get-Date), $PID
        $backupPath = "$Path.bak-$stamp"
        Copy-Item -Path $Path -Destination $backupPath -Force
        return $backupPath
    }
    return $null
}

function Write-Utf8NoBomFile($Path, $Value) {
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Value, $encoding)
}

function Claude-EnvObject($ApiKey = '{UserApiKey}') {
    [ordered]@{
        ANTHROPIC_BASE_URL = 'http://127.0.0.1:19099'
        ANTHROPIC_AUTH_TOKEN = $ApiKey
        ANTHROPIC_DEFAULT_HAIKU_MODEL = 'gpt-5.5'
        ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME = 'gpt-5.5'
        ANTHROPIC_DEFAULT_OPUS_MODEL = 'gpt-5.5'
        ANTHROPIC_DEFAULT_OPUS_MODEL_NAME = 'gpt-5.5'
        ANTHROPIC_DEFAULT_SONNET_MODEL = 'gpt-5.4[1M]'
        ANTHROPIC_DEFAULT_SONNET_MODEL_NAME = 'gpt-5.4'
        ANTHROPIC_MODEL = 'gpt-5.5'
        ENABLE_TOOL_SEARCH = 'true'
        DISABLE_AUTOUPDATER = '1'
    }
}

function Configure-Claude($Path, $ApiKey = '{UserApiKey}') {
    $backup = Backup-File $Path
    $dir = Split-Path -Parent $Path
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }

    $settings = [ordered]@{}
    if (Test-Path $Path) {
        try {
            $loaded = Read-JsonObject $Path
            foreach ($key in $loaded.Keys) {
                if ($key -ne 'env') { $settings[$key] = $loaded[$key] }
            }
        } catch {
            Write-Fail "Claude Code 配置文件不是合法 JSON：$Path"
        }
    }
    $settings['env'] = Claude-EnvObject $ApiKey
    $settings | ConvertTo-Json -Depth 20 | Set-Content -Path $Path -Encoding UTF8
    return ,@($Path, $backup)
}

function Opencode-ProviderObject($ApiKey = '{UserApiKey}') {
    [ordered]@{
        'mtls-router' = [ordered]@{
            npm = '@ai-sdk/openai-compatible'
            name = 'mtls-router'
            options = [ordered]@{
                baseURL = 'http://127.0.0.1:19099/v1'
                apiKey = $ApiKey
            }
            models = [ordered]@{
                'gpt-5.5' = [ordered]@{
                    name = 'GPT-5.5'
                    reasoning = $true
                    attachment = $true
                    tool_call = $true
                    limit = [ordered]@{ context = 272000; input = 244800; output = 27200 }
                    modalities = [ordered]@{ input = @('text','image'); output = @('text') }
                    options = [ordered]@{ reasoningEffort = 'medium' }
                }
                'gpt-5.4' = [ordered]@{
                    name = 'GPT-5.4'
                    reasoning = $true
                    attachment = $true
                    tool_call = $true
                    limit = [ordered]@{ context = 1000000; input = 900000; output = 100000 }
                    modalities = [ordered]@{ input = @('text','image'); output = @('text') }
                    options = [ordered]@{ reasoningEffort = 'medium' }
                }
            }
        }
    }
}

function ConvertFrom-JsoncText($Text) {
    if ($null -eq $Text) { return $null }

    $builder = [System.Text.StringBuilder]::new()
    $i = 0
    $inString = $false
    $escape = $false
    while ($i -lt $Text.Length) {
        $ch = $Text[$i]
        $next = if ($i + 1 -lt $Text.Length) { $Text[$i + 1] } else { [char]0 }
        if ($inString) {
            [void]$builder.Append($ch)
            if ($escape) { $escape = $false }
            elseif ($ch -eq '\') { $escape = $true }
            elseif ($ch -eq '"') { $inString = $false }
            $i++
            continue
        }
        if ($ch -eq '"') {
            $inString = $true
            [void]$builder.Append($ch)
            $i++
            continue
        }
        if ($ch -eq '/' -and $next -eq '/') {
            $i += 2
            while ($i -lt $Text.Length -and $Text[$i] -notin "`r", "`n") { $i++ }
            continue
        }
        if ($ch -eq '/' -and $next -eq '*') {
            $i += 2
            while ($i + 1 -lt $Text.Length -and -not ($Text[$i] -eq '*' -and $Text[$i + 1] -eq '/')) { $i++ }
            $i = [Math]::Min($i + 2, $Text.Length)
            continue
        }
        [void]$builder.Append($ch)
        $i++
    }

    $withoutComments = $builder.ToString()
    $builder = [System.Text.StringBuilder]::new()
    $i = 0
    $inString = $false
    $escape = $false
    while ($i -lt $withoutComments.Length) {
        $ch = $withoutComments[$i]
        if ($inString) {
            [void]$builder.Append($ch)
            if ($escape) { $escape = $false }
            elseif ($ch -eq '\') { $escape = $true }
            elseif ($ch -eq '"') { $inString = $false }
            $i++
            continue
        }
        if ($ch -eq '"') {
            $inString = $true
            [void]$builder.Append($ch)
            $i++
            continue
        }
        if ($ch -eq ',') {
            $j = $i + 1
            while ($j -lt $withoutComments.Length -and [char]::IsWhiteSpace($withoutComments[$j])) { $j++ }
            if ($j -lt $withoutComments.Length -and ($withoutComments[$j] -eq '}' -or $withoutComments[$j] -eq ']')) {
                $i++
                continue
            }
        }
        [void]$builder.Append($ch)
        $i++
    }
    return $builder.ToString()
}

function Convert-OpencodeJsoncToJson($JsoncPath, $JsonPath) {
    if ($JsoncPath -notlike '*.jsonc') {
        Write-Fail "opencode JSONC 源文件必须是 *.jsonc：$JsoncPath"
    }
    if (Test-Path $JsonPath) {
        Write-Fail "opencode JSON 目标文件已存在，拒绝覆盖：$JsonPath"
    }

    try {
        $clean = ConvertFrom-JsoncText (Get-Content $JsoncPath -Raw)
        $loaded = ConvertTo-Hashtable ($clean | ConvertFrom-Json)
    } catch {
        Write-Fail "opencode JSONC 配置文件清洗后不是合法 JSON：$JsoncPath"
    }

    if ($loaded -isnot [System.Collections.IDictionary]) {
        $actual = if ($null -eq $loaded) { 'null' } else { $loaded.GetType().Name }
        Write-Fail "opencode JSONC 根节点不是对象（实际为 $actual），无法迁移：$JsoncPath"
    }
    if ($loaded.Contains('provider') -and $null -ne $loaded['provider'] -and $loaded['provider'] -isnot [System.Collections.IDictionary]) {
        Write-Fail "opencode JSONC 现有 .provider 字段不是对象（实际为 $($loaded['provider'].GetType().Name)），无法迁移：$JsoncPath"
    }

    $backup = Backup-File $JsoncPath
    $dir = Split-Path -Parent $JsonPath
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    $loaded | ConvertTo-Json -Depth 30 | Set-Content -Path $JsonPath -Encoding UTF8
    return ,@($JsonPath, $backup)
}

function Configure-Opencode($Path, $ApiKey = '{UserApiKey}') {
    $backup = $null
    $sourcePath = $Path
    if ($Path -like '*.jsonc') {
        $Path = Join-Path (Split-Path -Parent $Path) 'opencode.json'
        if (-not (Test-Path $sourcePath) -and (Test-Path $Path)) {
            $sourcePath = $Path
        }
    }

    $dir = Split-Path -Parent $Path
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }

    $config = [ordered]@{}
    if (Test-Path $sourcePath) {
        try {
            $loaded = Read-JsonObject $sourcePath
            foreach ($key in $loaded.Keys) { $config[$key] = $loaded[$key] }
        } catch {
            if ($sourcePath -like '*.jsonc') {
                Write-Fail "opencode JSONC 配置文件清洗后不是合法 JSON：$sourcePath"
            }
            Write-Fail "opencode 配置文件不是合法 JSON：$sourcePath"
        }
        if ($config.Contains('provider') -and $null -ne $config['provider'] -and $config['provider'] -isnot [System.Collections.IDictionary]) {
            Write-Fail "opencode 现有 .provider 字段不是对象（实际为 $($config['provider'].GetType().Name)），无法合并：$sourcePath"
        }
        $backup = Backup-File $sourcePath
    }

    if (-not $config.Contains('provider') -or $null -eq $config['provider']) {
        $config['provider'] = [ordered]@{}
    }
    $config['provider']['mtls-router'] = (Opencode-ProviderObject $ApiKey)['mtls-router']
    $config | ConvertTo-Json -Depth 30 | Set-Content -Path $Path -Encoding UTF8
    return ,@($Path, $backup)
}

function Remove-CodexBlock($Path, $Header) {
    if (-not (Test-Path $Path)) { return }
    $lines = Get-Content $Path
    $result = New-Object System.Collections.Generic.List[string]
    $skip = $false
    foreach ($line in $lines) {
        if ($line -match "^\[$Header\]$") {
            $skip = $true
            continue
        }
        if ($skip -and $line -match '^\[') {
            $skip = $false
        }
        if (-not $skip) {
            $result.Add($line)
        }
    }
    Set-Content -Path $Path -Value $result -Encoding UTF8
}

function Remove-CodexRootKeys($Path) {
    if (-not (Test-Path $Path)) { return }
    $lines = Get-Content $Path
    $result = New-Object System.Collections.Generic.List[string]
    $inRoot = $true
    foreach ($line in $lines) {
        if ($line -match '^\s*\[') { $inRoot = $false }
        if ($inRoot -and $line -match '^\s*(model_provider|model|disable_response_storage)\s*=') {
            continue
        }
        $result.Add($line)
    }
    Set-Content -Path $Path -Value $result -Encoding UTF8
}

function Configure-Codex($Path, $apiKey = '') {
    $backup = Backup-File $Path
    $dir = Split-Path -Parent $Path
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    if (-not (Test-Path $Path)) { New-Item -ItemType File -Force -Path $Path | Out-Null }

    Remove-CodexRootKeys $Path
    Remove-CodexBlock $Path 'model_providers.custom'

    $bodyTmp = [System.IO.Path]::GetTempFileName()
    Copy-Item -Path $Path -Destination $bodyTmp -Force
    $header = @'
model_provider = "custom"
model = "gpt-5.5"
disable_response_storage = true

[model_providers.custom]
name = "9router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19099/v1"
'@
    $body = Get-Content $bodyTmp -Raw
    if ($body.Length -gt 0) {
        Set-Content -Path $Path -Value ($header + "`n" + $body) -Encoding UTF8
    } else {
        Set-Content -Path $Path -Value $header -Encoding UTF8
    }
    Remove-Item $bodyTmp -Force

    if ($apiKey) {
        $authPath = Join-Path (Split-Path -Parent $Path) 'auth.json'
        $authBackup = Backup-File $authPath
        $authDir = Split-Path -Parent $authPath
        if ($authDir) { New-Item -ItemType Directory -Force -Path $authDir | Out-Null }
        $authJson = [ordered]@{ OPENAI_API_KEY = $apiKey } | ConvertTo-Json -Depth 20
        Write-Utf8NoBomFile $authPath $authJson
        return ,@($Path, $backup, "AUTH:$authPath", $authBackup)
    }

    return ,@($Path, $backup)
}

function Print-NextSteps {
    Write-Success '============================================================'
    Write-Success '配置完成。'
    Write-Success '============================================================'
    if ($script:RouterStarted) {
        Write-Info 'mtls-router 已在后台运行：'
        Write-Info "  $RouterBaseUrl"
    } else {
        Write-Info '未启动 mtls-router（本次仅处理配置）。如需启动，请运行：'
        Write-Info "  $PSCommandPath"
    }
    if ($script:ConfiguredAgentPaths.Count -gt 0) {
        Write-Info '已写入配置：'
        foreach ($line in $script:ConfiguredAgentPaths) {
            if ($line) { Write-Info "  $line" }
        }
    } else {
        Write-Info '未写入任何 agent 配置。'
    }
    if ($script:ConfiguredBackups.Count -gt 0) {
        Write-Info '已备份：'
        foreach ($line in $script:ConfiguredBackups) {
            if ($line) { Write-Info "  $line" }
        }
    }
    Write-Info '现在可以手动启动 agent。'
}


function ConvertTo-AgentKey($DisplayName) {
    switch ($DisplayName) {
        'Claude Code' { return 'claude' }
        'opencode' { return 'opencode' }
        'Codex' { return 'codex' }
        default { return $null }
    }
}

function ConvertFrom-AgentKey($Key) {
    switch ($Key) {
        'claude' { return 'Claude Code' }
        'opencode' { return 'opencode' }
        'codex' { return 'Codex' }
        default { return $null }
    }
}

function Show-Usage {
    @'
用法: setup.ps1 [router|agent] <command> [选项]

默认行为：setup.ps1 等价于 setup.ps1 router setup，下载并启动 mtls-router，**不会修改任何 agent 配置文件**。

子命令:
  router install
      只下载/安装 mtls-router。
  router start
      只启动已安装的 mtls-router，并记录 setup-managed 状态；不存在时提示先 install 或 setup。
  router setup
      下载/安装并启动 mtls-router。
  router status
      显示 setup-managed router 是否仍在运行，以及 pid/listen/binary/log 信息。
  router log [--tail=N]
      打印 setup-managed router 日志，默认显示最后 200 行。
  router stop
      停止 setup-managed router 进程。
  agent print-config [--agent=claude,opencode,codex]
      把要写入的配置片段打印到 stdout（只输出，不动文件）。默认所有检测到的 agent。
  agent write-config --agent=claude,opencode,codex
      把 mtls-router 配置写入检测到的 agent 配置文件。会先备份原文件。
      需要先 --agent= 指定至少一个，否则报错。

兼容旧参数:
  --print-config [--agent=claude,opencode,codex]
      等价于 agent print-config。
  --write-config --agent=claude,opencode,codex
      等价于 agent write-config --agent=...。

选项:
  --agent=LIST
      逗号分隔的 agent key：claude / opencode / codex。
  --download-url=URL
      自定义 mtls-router 下载地址。可指向包含二进制的目录，也可指向当前平台完整二进制 URL。
      也可用 MTLS_ROUTER_DOWNLOAD_URL 设置。
  --download-user=USER
      下载自定义 URL 时使用的 HTTP Basic Auth 用户名；也可用 MTLS_ROUTER_DOWNLOAD_USER 设置。
  --download-password=PASSWORD
      下载自定义 URL 时使用的 HTTP Basic Auth 密码；也可用 MTLS_ROUTER_DOWNLOAD_PASSWORD 设置。
  --version=VERSION
      自定义下载目录下的版本子目录名；默认 latest。也可用 MTLS_ROUTER_VERSION 设置。
  -h, --help
      显示本帮助。

示例:
  # 只下载并启动 router，不动 agent 配置
  .\setup.ps1 router setup

  # 管理后台 router
  .\setup.ps1 router status
  .\setup.ps1 router log
  .\setup.ps1 router stop

  # 只安装 router
  .\setup.ps1 router install

  # 只启动已安装 router
  .\setup.ps1 router start

  # 看看会写入哪些内容（不动文件）
  .\setup.ps1 agent print-config

  # 只为 Claude Code 写入配置
  .\setup.ps1 agent write-config --agent=claude
'@
}

function Main {
    $action = 'setup'
    $agentFilter = ''

    $routerLogLines = 200

    if ($args.Count -gt 0) {
        switch ($args[0]) {
            'router' {
                if ($args.Count -lt 2) { Write-Fail 'router 需要子命令：install / start / setup / status / log / stop（试试 --help）' }
                switch ($args[1]) {
                    'install' { $action = 'router-install' }
                    'start' { $action = 'router-start' }
                    'setup' { $action = 'router-setup' }
                    'status' { $action = 'router-status' }
                    'log' { $action = 'router-log' }
                    'stop' { $action = 'router-stop' }
                    default { Write-Fail "未知 router 子命令：$($args[1])（可用：install / start / setup / status / log / stop）" }
                }
                $args = @($args | Select-Object -Skip 2)
            }
            'agent' {
                if ($args.Count -lt 2) { Write-Fail 'agent 需要子命令：print-config / write-config（试试 --help）' }
                switch ($args[1]) {
                    'print-config' { $action = 'print' }
                    'write-config' { $action = 'write' }
                    default { Write-Fail "未知 agent 子命令：$($args[1])（可用：print-config / write-config）" }
                }
                $args = @($args | Select-Object -Skip 2)
            }
            '--print-config' {
                $action = 'print'
                $args = @($args | Select-Object -Skip 1)
            }
            '--write-config' {
                $action = 'write'
                $args = @($args | Select-Object -Skip 1)
            }
            '-h' { Show-Usage; return }
            '--help' { Show-Usage; return }
            default {
                if ($args[0] -notlike '--*') { Write-Fail "Unknown argument: $($args[0]) (try --help)" }
            }
        }
    }

    $i = 0
    while ($i -lt $args.Count) {
        $a = $args[$i]
        switch -Regex ($a) {
            '^--print-config$' {
                if ($action -ne 'setup') { Write-Fail '--print-config 只能作为兼容旧参数在顶层使用' }
                $action = 'print'
                $i++
                continue
            }
            '^--write-config$' {
                if ($action -ne 'setup') { Write-Fail '--write-config 只能作为兼容旧参数在顶层使用' }
                $action = 'write'
                $i++
                continue
            }
            '^--tail=(.+)$' {
                $routerLogLines = $Matches[1]
                $i++
                continue
            }
            '^--tail$' {
                if ($i + 1 -ge $args.Count) { Write-Fail '--tail needs a line count' }
                $routerLogLines = $args[$i + 1]
                $i += 2
                continue
            }
            '^--agent=(.+)$' {
                $agentFilter = $Matches[1]
                $i++
                continue
            }
            '^--agent$' {
                if ($i + 1 -ge $args.Count) { Write-Fail '--agent needs a value (comma-separated list)' }
                $agentFilter = $args[$i + 1]
                $i += 2
                continue
            }
            '^--download-url=(.+)$' {
                $script:DownloadBaseUrl = $Matches[1]
                $i++
                continue
            }
            '^--download-url$' {
                if ($i + 1 -ge $args.Count) { Write-Fail '--download-url needs a URL' }
                $script:DownloadBaseUrl = $args[$i + 1]
                $i += 2
                continue
            }
            '^--download-user=(.*)$' {
                $script:DownloadUser = $Matches[1]
                $i++
                continue
            }
            '^--download-user$' {
                if ($i + 1 -ge $args.Count) { Write-Fail '--download-user needs a username' }
                $script:DownloadUser = $args[$i + 1]
                $i += 2
                continue
            }
            '^--download-password=(.*)$' {
                $script:DownloadPassword = $Matches[1]
                $i++
                continue
            }
            '^--download-password$' {
                if ($i + 1 -ge $args.Count) { Write-Fail '--download-password needs a password' }
                $script:DownloadPassword = $args[$i + 1]
                $i += 2
                continue
            }
            '^--version=(.+)$' {
                $env:MTLS_ROUTER_VERSION = $Matches[1]
                $i++
                continue
            }
            '^--version$' {
                if ($i + 1 -ge $args.Count) { Write-Fail '--version needs a value' }
                $env:MTLS_ROUTER_VERSION = $args[$i + 1]
                $i += 2
                continue
            }
            '^(-h|--help)$' {
                Show-Usage
                return
            }
            default { Write-Fail "Unknown argument: $a (try --help)" }
        }
    }

    Show-Banner

    switch ($action) {
        'setup' {
            if ($env:MTLS_ROUTER_SKIP_DOWNLOAD -eq '1') {
                Write-Info '[Download] skipped (MTLS_ROUTER_SKIP_DOWNLOAD=1)'
            } else {
                Download-MtlsRouter
            }
            Start-MtlsRouter
            if ($env:MTLS_ROUTER_SKIP_START -ne '1') {
                $script:RouterStarted = $true
            }
            Write-Info '提示：未对 agent 配置做任何改动。如需写入 mtls-router 配置：'
            Write-Info "  $PSCommandPath agent write-config --agent=claude,opencode,codex"
            Write-Info "先看会写什么：$PSCommandPath agent print-config"
            if ($env:MTLS_ROUTER_SKIP_START -eq '1') {
                Write-Info '（已跳过实际启动 mtls-router）'
            }
            return
        }
        'router-setup' {
            if ($env:MTLS_ROUTER_SKIP_DOWNLOAD -eq '1') {
                Write-Info '[Download] skipped (MTLS_ROUTER_SKIP_DOWNLOAD=1)'
            } else {
                Download-MtlsRouter
            }
            Start-MtlsRouter
            if ($env:MTLS_ROUTER_SKIP_START -ne '1') {
                $script:RouterStarted = $true
            }
            Write-Info '提示：未对 agent 配置做任何改动。如需写入 mtls-router 配置：'
            Write-Info "  $PSCommandPath agent write-config --agent=claude,opencode,codex"
            Write-Info "先看会写什么：$PSCommandPath agent print-config"
            if ($env:MTLS_ROUTER_SKIP_START -eq '1') {
                Write-Info '（已跳过实际启动 mtls-router）'
            }
            return
        }
        'router-install' {
            if ($env:MTLS_ROUTER_SKIP_DOWNLOAD -eq '1') {
                Write-Info '[Download] skipped (MTLS_ROUTER_SKIP_DOWNLOAD=1)'
            } else {
                Download-MtlsRouter
            }
            Print-NextSteps
            return
        }
        'router-start' {
            Start-MtlsRouter
            if ($env:MTLS_ROUTER_SKIP_START -ne '1') {
                $script:RouterStarted = $true
            }
            Print-NextSteps
            return
        }
        'router-status' { Show-RouterStatus; return }
        'router-log' { Show-RouterLog $routerLogLines; return }
        'router-stop' { Stop-Router; return }
    }

    Detect-Agents

    if ([string]::IsNullOrWhiteSpace($agentFilter) -and $action -eq 'write') {
        Write-Fail '--write-config 需要 --agent=claude,opencode,codex 显式指定要操作的 agent。'
    }

    $targets = New-Object System.Collections.Generic.List[string]

    if ([string]::IsNullOrWhiteSpace($agentFilter)) {
        if ($DetectedAgents.Count -eq 0) {
            Write-Warn '  未检测到 Claude Code、opencode 或 Codex。'
            Write-Info '提示：用 --agent=claude,opencode,codex 显式指定目标。'
            return
        }
        foreach ($agent in $DetectedAgents) {
            $k = ConvertTo-AgentKey $agent.Name
            if ($k) { $targets.Add($k) }
        }
    } else {
        $tokens = $agentFilter -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ }
        $seen = @{}
        foreach ($token in $tokens) {
            if ($seen.ContainsKey($token)) { Write-Fail "--agent 列表中有重复项: $token" }
            $seen[$token] = $true

            $idx = -1
            for ($j = 0; $j -lt $DetectedAgents.Count; $j++) {
                if ((ConvertTo-AgentKey $DetectedAgents[$j].Name) -eq $token) {
                    $idx = $j
                    break
                }
            }
            if ($idx -lt 0) {
                $detected = if ($DetectedAgents.Count -gt 0) {
                    ($DetectedAgents | ForEach-Object { $_.Name }) -join ' '
                } else { '(无)' }
                Write-Fail "未检测到 agent: $token (已检测: $detected)"
            }
            $targets.Add($token)
        }
    }

    $script:ConfiguredAgentPaths = @()
    $script:ConfiguredBackups = @()

    $sharedApiKey = ''
    if ($action -eq 'write') {
        $needsApiKey = $false
        foreach ($k in $targets) {
            if ($k -in @('claude', 'opencode', 'codex')) {
                $needsApiKey = $true
                break
            }
        }
        if ($needsApiKey) {
            $sharedApiKey = $env:MTLS_ROUTER_OPENAI_API_KEY
            if (-not $sharedApiKey) {
                try {
                    $secure = Read-Host '请输入 mtls-router OPENAI_API_KEY（输入隐藏）' -AsSecureString
                    $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
                    $sharedApiKey = [System.Runtime.InteropServices.Marshal]::PtrToStringAuto($bstr)
                    [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) | Out-Null
                } catch {
                    $sharedApiKey = ''
                }
            }
            if (-not $sharedApiKey) {
                Write-Fail '写入 claude/opencode/codex 配置需要 apikey。在 TTY 下重试，或通过 MTLS_ROUTER_OPENAI_API_KEY 环境变量传入。'
            }
        }
    }

    foreach ($key in $targets) {
        $displayName = ConvertFrom-AgentKey $key
        if (-not $displayName) { Write-Fail "Unknown agent key: $key" }

        $idx = -1
        for ($j = 0; $j -lt $DetectedAgents.Count; $j++) {
            if ((ConvertTo-AgentKey $DetectedAgents[$j].Name) -eq $key) {
                $idx = $j
                break
            }
        }
        if ($idx -lt 0) { Write-Fail "未检测到 agent: $key" }
        $path = $DetectedAgents[$idx].ConfigPath

        if ($action -eq 'print') {
            Write-Host ''
            Write-Host "### $displayName -> $path"
            switch ($key) {
                'claude' {
                    Write-Host '### 把以下片段合并到现有 settings.json 中（保留其他字段）：'
                    Write-Host ''
                    $env = Claude-EnvObject | ConvertTo-Json -Depth 20
                    Write-Host "{`n  `"env`": $env`n}"
                }
                'opencode' {
                    Write-Host '### 把以下片段合并到 .provider 字段：'
                    Write-Host ''
                    Write-Host (Opencode-ProviderObject | ConvertTo-Json -Depth 20)
                }
                'codex' {
                    $authPath = Join-Path (Split-Path -Parent $path) 'auth.json'
                    Write-Host '### 使用以下最小 TOML 配置 config.toml：'
                    Write-Host ''
                    @'
model_provider = "custom"
model = "gpt-5.5"
disable_response_storage = true

[model_providers.custom]
name = "9router"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:19099/v1"
'@
                    Write-Host ''
                    Write-Host "### $displayName -> $authPath"
                    Write-Host '### 将 auth.json 覆盖为以下最小 JSON:'
                    Write-Host ''
                    Write-Host '{'
                    Write-Host '  "OPENAI_API_KEY": "{UserApiKey}"'
                    Write-Host '}'
                }
            }
            continue
        }

        $result = switch ($key) {
            'claude' { Configure-Claude $path $sharedApiKey }
            'opencode' {
                if ($path -like '*.jsonc') {
                    $jsonPath = Join-Path (Split-Path -Parent $path) 'opencode.json'
                    if (-not [Console]::IsInputRedirected -and -not (Test-Path $jsonPath)) {
                        Write-Warn "检测到 opencode 当前配置为 JSONC：$path"
                        Write-Warn 'setup 暂不支持就地合并 JSONC。'
                        $answer = Read-Host '是否尝试备份该 JSONC，并迁移为标准 JSON opencode.json 后写入 mtls-router provider？[y/N]'
                        if ($answer -in @('y', 'Y', 'yes', 'YES')) {
                            $migration = Convert-OpencodeJsoncToJson $path $jsonPath
                            $path = $migration[0]
                            $migrationBackup = if ($migration.Count -gt 1) { $migration[1] } else { $null }
                            if ($migrationBackup) { $script:ConfiguredBackups += $migrationBackup }
                            Write-Success "  已从 opencode.jsonc 迁移到 opencode.json：$path"
                            Write-Warn "  注意：JSONC 注释和原格式不会保留，原文件已备份为 $migrationBackup"
                        } else {
                            Write-Warn '  已跳过 opencode 写入。可手动创建标准 JSON，或设置 OPENCODE_CONFIG 指向 JSON 文件。'
                            continue
                        }
                    }
                }
                Configure-Opencode $path $sharedApiKey
            }
            'codex' { Configure-Codex $path $sharedApiKey }
            default { Write-Fail "Unknown agent key: $key" }
        }

        $wrote = $result[0]
        $backup = if ($result.Count -gt 1) { $result[1] } else { $null }
        $script:ConfiguredAgentPaths += ("{0}: {1}" -f $displayName, $wrote)
        if ($backup) { $script:ConfiguredBackups += $backup }
        Write-Success ("  已写入 {0} 配置: {1}" -f $displayName, $wrote)
        if ($backup) { Write-Success "  备份: $backup" }
        # codex also writes auth.json when an api key was provided.
        if ($key -eq 'codex' -and $result.Count -ge 3 -and $result[2] -like 'AUTH:*') {
            $authPath = $result[2] -replace '^AUTH:', ''
            $authBackup = if ($result.Count -ge 4) { $result[3] } else { $null }
            $script:ConfiguredAgentPaths += ("{0} auth: {1}" -f $displayName, $authPath)
            if ($authBackup) { $script:ConfiguredBackups += $authBackup }
            Write-Success ("  已写入 {0} auth.json: {1}" -f $displayName, $authPath)
            if ($authBackup) { Write-Success "  备份: $authBackup" }
        }
    }

    if ($action -eq 'print') {
        return
    }

    Print-NextSteps
}

Main @args
