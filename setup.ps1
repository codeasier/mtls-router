$ErrorActionPreference = 'Stop'

$Repo = 'codeasier/mtls-router'
$RouterBaseUrl = 'http://127.0.0.1:19099'
$InstallDir = if ($env:MTLS_ROUTER_INSTALL_DIR) { $env:MTLS_ROUTER_INSTALL_DIR } else { Join-Path $env:USERPROFILE '.local\bin' }
$BinaryPath = Join-Path $InstallDir 'mtls-router.exe'
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

function Download-MtlsRouter {
    Write-Info '[下载] 检测并下载最新 mtls-router...'
    $asset = Get-RouterAssetName

    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'mtls-router-setup' }
    } catch {
        Write-Fail "无法获取 GitHub 最新 release：$($_.Exception.Message)"
    }

    $version = $release.tag_name
    if (-not $version) { Write-Fail '无法读取最新 release 版本。' }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $url = "https://github.com/$Repo/releases/download/$version/$asset"

    Write-Info "  版本：$version"
    Write-Info "  平台：$asset"
    Write-Info "  安装：$BinaryPath"

    try {
        Invoke-WebRequest -Uri $url -OutFile $BinaryPath -Headers @{ 'User-Agent' = 'mtls-router-setup' }
    } catch {
        Write-Fail "下载 mtls-router 失败：$($_.Exception.Message)"
    }

    Write-Success "  已安装 mtls-router：$BinaryPath"
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

function Claude-EnvObject {
    [ordered]@{
        ANTHROPIC_BASE_URL = 'http://127.0.0.1:19099'
        ANTHROPIC_AUTH_TOKEN = '{UserApiKey}'
        ANTHROPIC_DEFAULT_HAIKU_MODEL = 'cx/gpt-5.5'
        ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME = 'gpt-5.5'
        ANTHROPIC_DEFAULT_OPUS_MODEL = 'cx/gpt-5.5'
        ANTHROPIC_DEFAULT_OPUS_MODEL_NAME = 'gpt-5.5'
        ANTHROPIC_DEFAULT_SONNET_MODEL = 'cx/gpt-5.4[1M]'
        ANTHROPIC_DEFAULT_SONNET_MODEL_NAME = 'gpt-5.4'
        ANTHROPIC_MODEL = 'cx/gpt-5.5'
        ENABLE_TOOL_SEARCH = 'true'
        DISABLE_AUTOUPDATER = '1'
    }
}

function Configure-Claude($Path) {
    $backup = Backup-File $Path
    $dir = Split-Path -Parent $Path
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }

    $settings = [ordered]@{}
    if (Test-Path $Path) {
        try {
            $loaded = Get-Content $Path -Raw | ConvertFrom-Json -AsHashtable
            foreach ($key in $loaded.Keys) {
                if ($key -ne 'env') { $settings[$key] = $loaded[$key] }
            }
        } catch {
            Write-Fail "Claude Code 配置文件不是合法 JSON：$Path"
        }
    }
    $settings['env'] = Claude-EnvObject
    $settings | ConvertTo-Json -Depth 20 | Set-Content -Path $Path -Encoding UTF8
    return ,@($Path, $backup)
}

function Opencode-ProviderObject {
    [ordered]@{
        'mtls-router' = [ordered]@{
            npm = '@ai-sdk/openai-compatible'
            name = 'mtls-router'
            options = [ordered]@{
                baseURL = 'http://127.0.0.1:19099'
                apiKey = '{UserApiKey}'
            }
            models = [ordered]@{
                'cx/gpt-5.5' = [ordered]@{
                    name = 'GPT-5.5'
                    reasoning = $true
                    attachment = $true
                    tool_call = $true
                    limit = [ordered]@{ context = 272000; input = 244800; output = 27200 }
                    modalities = [ordered]@{ input = @('text','image'); output = @('text') }
                    options = [ordered]@{ reasoningEffort = 'medium' }
                }
                'cx/gpt-5.4' = [ordered]@{
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

function Configure-Opencode($Path) {
    if ($Path -like '*.jsonc') {
        Write-Fail "opencode 当前选中的配置文件是 JSONC：$Path（暂不支持就地合并）。请设置 OPENCODE_CONFIG 指向 JSON 文件。"
    }

    $backup = Backup-File $Path
    $dir = Split-Path -Parent $Path
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }

    $config = [ordered]@{}
    if (Test-Path $Path) {
        try {
            $loaded = Get-Content $Path -Raw | ConvertFrom-Json -AsHashtable
            foreach ($key in $loaded.Keys) { $config[$key] = $loaded[$key] }
        } catch {
            Write-Fail "opencode 配置文件不是合法 JSON：$Path"
        }
        if ($config.Contains('provider') -and $null -ne $config['provider'] -and $config['provider'] -isnot [System.Collections.IDictionary]) {
            Write-Fail "opencode 现有 .provider 字段不是对象（实际为 $($config['provider'].GetType().Name)），无法合并：$Path"
        }
    }

    if (-not $config.Contains('provider') -or $null -eq $config['provider']) {
        $config['provider'] = [ordered]@{}
    }
    $config['provider']['mtls-router'] = (Opencode-ProviderObject)['mtls-router']
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

function Configure-Codex($Path, $apiKey = '') {
    $backup = Backup-File $Path
    $dir = Split-Path -Parent $Path
    if ($dir) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    if (-not (Test-Path $Path)) { New-Item -ItemType File -Force -Path $Path | Out-Null }

    Remove-CodexBlock $Path 'model_providers.mtls-router'
    Remove-CodexBlock $Path 'profiles.gpt-5-5-router'
    Remove-CodexBlock $Path 'profiles.gpt-5-4-1m-router'

    Add-Content -Path $Path -Encoding UTF8 -Value @'

# mtls-router provider
[model_providers.mtls-router]
name = "mtls-router"
base_url = "http://127.0.0.1:19099/v1"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
request_max_retries = 2
stream_max_retries = 2
supports_websockets = false

# GPT-5.5 via mtls-router
[profiles.gpt-5-5-router]
model = "gpt-5.5"
model_provider = "mtls-router"
model_reasoning_effort = "medium"

# GPT-5.4 1M via mtls-router
[profiles.gpt-5-4-1m-router]
model = "gpt-5.4"
model_provider = "mtls-router"
model_reasoning_effort = "medium"
'@

    if ($apiKey) {
        $authPath = Join-Path (Split-Path -Parent $Path) 'auth.json'
        $authBackup = Backup-File $authPath
        $authDir = Split-Path -Parent $authPath
        if ($authDir) { New-Item -ItemType Directory -Force -Path $authDir | Out-Null }
        $auth = [ordered]@{}
        if (Test-Path $authPath) {
            try {
                $loaded = Get-Content $authPath -Raw | ConvertFrom-Json -AsHashtable
                foreach ($key in $loaded.Keys) { $auth[$key] = $loaded[$key] }
            } catch {
                Write-Fail "Codex auth.json 不是合法 JSON：$authPath"
            }
        }
        $auth['OPENAI_API_KEY'] = $apiKey
        $auth | ConvertTo-Json -Depth 20 | Set-Content -Path $authPath -Encoding UTF8
        return ,@($Path, $backup, "AUTH:$authPath", $authBackup)
    }

    return ,@($Path, $backup)
}

function Print-NextSteps {
    Write-Success '============================================================'
    Write-Success '配置完成。'
    Write-Success '============================================================'
    Write-Info 'mtls-router 已在后台运行：'
    Write-Info "  $RouterBaseUrl"
    Write-Info "  日志文件: $script:LogPath"
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
用法: setup.ps1 [选项]

默认行为：下载并启动 mtls-router，**不会修改任何 agent 配置文件**。

选项:
  --print-config [--agent=claude,opencode,codex]
      把要写入的配置片段打印到 stdout（只输出，不动文件）。默认所有检测到的 agent。
  --write-config [--agent=claude,opencode,codex]
      把 mtls-router 配置写入检测到的 agent 配置文件。会先备份原文件。
      需要先 --agent= 指定至少一个，否则报错。
  --agent=LIST
      逗号分隔的 agent key：claude / opencode / codex。
      与 --print-config 或 --write-config 搭配使用。
  -h, --help
      显示本帮助。

示例:
  # 只下载并启动 router，不动 agent 配置
  .\setup.ps1

  # 看看会写入哪些内容（不动文件）
  .\setup.ps1 --print-config

  # 只为 Claude Code 写入配置
  .\setup.ps1 --write-config --agent=claude

  # 为 opencode 和 Codex 写入配置
  .\setup.ps1 --write-config --agent=opencode,codex
'@
}

function Main {
    $action = 'start'
    $agentFilter = ''

    $i = 0
    while ($i -lt $args.Count) {
        $a = $args[$i]
        switch -Regex ($a) {
            '^--print-config$' {
                $action = 'print'
                $i++
                continue
            }
            '^--write-config$' {
                $action = 'write'
                $i++
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
            '^(-h|--help)$' {
                Show-Usage
                return
            }
            default { Write-Fail "Unknown argument: $a (try --help)" }
        }
    }

    Show-Banner

    if ($action -eq 'start') {
        if ($env:MTLS_ROUTER_SKIP_DOWNLOAD -eq '1') {
            Write-Info '[Download] skipped (MTLS_ROUTER_SKIP_DOWNLOAD=1)'
        } else {
            Download-MtlsRouter
        }

        if ($env:MTLS_ROUTER_SKIP_START -eq '1') {
            Write-Info '[Start] skipped (MTLS_ROUTER_SKIP_START=1)'
        } else {
            Start-MtlsRouter
        }

        Write-Info '提示：未对 agent 配置做任何改动。如需写入 mtls-router 配置：'
        Write-Info "  $PSCommandPath --write-config --agent=claude,opencode,codex"
        Write-Info "先看会写什么：$PSCommandPath --print-config"
        if ($env:MTLS_ROUTER_SKIP_START -eq '1') {
            Write-Info '（已跳过实际启动 mtls-router）'
        }
        return
    }

    if ($action -eq 'write') {
        if ($env:MTLS_ROUTER_SKIP_DOWNLOAD -eq '1') {
            Write-Info '[Download] skipped (MTLS_ROUTER_SKIP_DOWNLOAD=1)'
        } else {
            Download-MtlsRouter
        }

        if ($env:MTLS_ROUTER_SKIP_START -eq '1') {
            Write-Info '[Start] skipped (MTLS_ROUTER_SKIP_START=1)'
        } else {
            Start-MtlsRouter
        }
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
                    Write-Host '### 把以下 TOML 追加到 config.toml：'
                    Write-Host ''
                    @'

# mtls-router provider
[model_providers.mtls-router]
name = "mtls-router"
base_url = "http://127.0.0.1:19099/v1"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
request_max_retries = 2
stream_max_retries = 2
supports_websockets = false

# GPT-5.5 via mtls-router
[profiles.gpt-5-5-router]
model = "gpt-5.5"
model_provider = "mtls-router"
model_reasoning_effort = "medium"

# GPT-5.4 1M via mtls-router
[profiles.gpt-5-4-1m-router]
model = "gpt-5.4"
model_provider = "mtls-router"
model_reasoning_effort = "medium"
'@
                    Write-Host ''
                    Write-Host "### $displayName -> $authPath"
                    Write-Host '### 把以下 JSON 合并到 auth.json:'
                    Write-Host ''
                    Write-Host '{'
                    Write-Host '  "OPENAI_API_KEY": "{UserApiKey}"'
                    Write-Host '}'
                }
            }
            continue
        }

        $apiKey = $env:MTLS_ROUTER_OPENAI_API_KEY
        if (-not $apiKey) {
            try {
                $secure = Read-Host '请输入 mtls-router OPENAI_API_KEY（codex 不会自己管理这个 key）' -AsSecureString
                $bstr = [System.Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
                $apiKey = [System.Runtime.InteropServices.Marshal]::PtrToStringAuto($bstr)
                [System.Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) | Out-Null
            } catch {
                $apiKey = ''
            }
        }
        if ($key -eq 'codex' -and -not $apiKey) {
            Write-Fail 'codex 配置需要 apikey。在 TTY 下重试，或通过 MTLS_ROUTER_OPENAI_API_KEY 环境变量传入。'
        }

        $result = switch ($key) {
            'claude' { Configure-Claude $path }
            'opencode' { Configure-Opencode $path }
            'codex' { Configure-Codex $path $apiKey }
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
