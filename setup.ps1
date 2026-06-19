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
    if ($codex -and $codex.Source) {
        $configPath = if ($env:CODEX_HOME) {
            Join-Path $env:CODEX_HOME 'config.toml'
        } else {
            Join-Path (Join-Path $env:USERPROFILE '.codex') 'config.toml'
        }
        $script:DetectedAgents += [PSCustomObject]@{
            Name = 'Codex'
            Command = $codex.Source
            ConfigPath = $configPath
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

function Configure-Codex($Path) {
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
env_key = "{UserApiKey}"
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

    return ,@($Path, $backup)
}

function Print-NextSteps {
    Write-Success '============================================================'
    Write-Success '配置完成。'
    Write-Success '============================================================'
    Write-Info 'mtls-router 已在后台运行：'
    Write-Info "  $RouterBaseUrl"
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
    Write-Info '可手动启动 agent。'
}

function Main {
    Show-Banner
    Download-MtlsRouter
    Start-MtlsRouter
    Detect-Agents

    $script:ConfiguredAgentPaths = @()
    $script:ConfiguredBackups = @()

    if ($DetectedAgents.Count -eq 0) {
        Write-Warn '  未检测到 Claude Code、opencode 或 Codex。mtls-router 已启动，但未写入 agent 配置。'
    } else {
        $selection = ''
        $total = $DetectedAgents.Count
        if ($total -eq 1) {
            Write-Host ''
            Write-Host ("检测到 {0}：{1}" -f $DetectedAgents[0].Name, $DetectedAgents[0].Command)
            Write-Host ("配置文件：{0}" -f $DetectedAgents[0].ConfigPath)
            $reply = Read-Host '是否备份并写入配置？[y/N]'
            if ($reply -match '^[Yy]$') { $selection = '1' }
        } else {
            Write-Host ''
            Write-Host '检测到多个 Agent：'
            Write-Host '0) 全部覆盖配置'
            for ($i = 0; $i -lt $total; $i++) {
                $item = $DetectedAgents[$i]
                Write-Host (("{0}) {1}: {2} -> {3}") -f ($i + 1), $item.Name, $item.Command, $item.ConfigPath)
            }
            $selection = Read-Host '请输入编号，多个用空格分隔；直接回车则逐个询问：'
            if ([string]::IsNullOrWhiteSpace($selection)) {
                $selected = New-Object System.Collections.Generic.List[string]
                for ($i = 0; $i -lt $total; $i++) {
                    $item = $DetectedAgents[$i]
                    Write-Host ''
                    Write-Host ("检测到 {0}：{1}" -f $item.Name, $item.Command)
                    Write-Host ("配置文件：{0}" -f $item.ConfigPath)
                    $answer = Read-Host '是否备份并写入配置？[y/N]'
                    if ($answer -match '^[Yy]$') {
                        $selected.Add([string]($i + 1))
                    }
                }
                $selection = ($selected -join ' ')
            }
        }

        $chosen = Select-Targets $selection $total
        if ($chosen.Count -eq 0) {
            Write-Warn '  未选择任何 agent，跳过 agent 配置。'
        } else {
            foreach ($token in $chosen) {
                $idx = [int]$token - 1
                $agent = $DetectedAgents[$idx]
                $result = if ($agent.Name -eq 'Claude Code') {
                    Configure-Claude $agent.ConfigPath
                } elseif ($agent.Name -eq 'opencode') {
                    Configure-Opencode $agent.ConfigPath
                } elseif ($agent.Name -eq 'Codex') {
                    Configure-Codex $agent.ConfigPath
                } else {
                    Write-Fail "未知 agent：$($agent.Name)"
                }

                $wrote = $result[0]
                $backup = if ($result.Count -gt 1) { $result[1] } else { $null }
                $script:ConfiguredAgentPaths += ("{0}: {1}" -f $agent.Name, $wrote)
                if ($backup) { $script:ConfiguredBackups += $backup }
                Write-Success ("  已写入 {0} 配置：{1}" -f $agent.Name, $wrote)
            }
        }
    }

    Print-NextSteps
}

Main
