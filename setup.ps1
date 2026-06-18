$ErrorActionPreference = 'Stop'

$Repo = 'codeasier/mtls-router'
$RouterBaseUrl = 'http://127.0.0.1:19099'
$InstallDir = if ($env:MTLS_ROUTER_INSTALL_DIR) { $env:MTLS_ROUTER_INSTALL_DIR } else { Join-Path $env:USERPROFILE '.local\bin' }
$BinaryPath = Join-Path $InstallDir 'mtls-router.exe'

function Write-Info($Message) { Write-Host $Message -ForegroundColor Cyan }
function Write-Success($Message) { Write-Host $Message -ForegroundColor Green }
function Write-Warn($Message) { Write-Host $Message -ForegroundColor Yellow }
function Write-Fail($Message) { Write-Host $Message -ForegroundColor Red; exit 1 }

function Show-Banner {
    Write-Info '============================================================'
    Write-Info ' mtls-router Claude Code 一键配置工具'
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

function Ensure-ClaudeCode {
    Write-Info '[检测] Claude Code 安装状态...'
    $claude = Get-Command claude -ErrorAction SilentlyContinue
    if ($claude) {
        Write-Success "  已找到 Claude Code：$($claude.Source)"
        return
    }

    $localClaude = Join-Path $env:USERPROFILE '.local\bin\claude.exe'
    if (Test-Path $localClaude) {
        $env:Path = "$(Split-Path $localClaude);$env:Path"
        Write-Success "  已找到 Claude Code：$localClaude"
        return
    }

    Write-Warn '  未找到 Claude Code，开始尝试通过 npm 安装...'
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
        Write-Fail '未找到 npm。请先安装 Node.js/npm 后重试。'
    }

    npm install -g '@anthropic-ai/claude-code' --registry 'https://registry.npmmirror.com'
    Refresh-Path

    $claude = Get-Command claude -ErrorAction SilentlyContinue
    if (-not $claude) { Write-Fail 'Claude Code 安装后仍不可用，请检查 npm 全局 bin 是否在 PATH 中。' }
    Write-Success "  Claude Code 安装完成：$($claude.Source)"
}

function Set-ObjectProperty($Object, $Name, $Value) {
    $property = $Object.PSObject.Properties[$Name]
    if ($property) {
        $property.Value = $Value
    } else {
        $Object | Add-Member -MemberType NoteProperty -Name $Name -Value $Value
    }
}

function Update-ClaudeSettings {
    Write-Info '[修复] 写入 Claude Code settings.json...'
    $claudeDir = Join-Path $env:USERPROFILE '.claude'
    $settingsPath = Join-Path $claudeDir 'settings.json'
    New-Item -ItemType Directory -Force -Path $claudeDir | Out-Null

    $settings = New-Object PSObject
    if (Test-Path $settingsPath) {
        try {
            $settings = Get-Content $settingsPath -Raw | ConvertFrom-Json
        } catch {
            $settings = New-Object PSObject
        }
    }

    $envSettings = $settings.PSObject.Properties['env'].Value
    if (-not $envSettings -or $envSettings -isnot [PSCustomObject]) {
        $envSettings = New-Object PSObject
        Set-ObjectProperty $settings 'env' $envSettings
    }
    Set-ObjectProperty $envSettings 'ANTHROPIC_BASE_URL' $RouterBaseUrl

    $settings | ConvertTo-Json -Depth 20 | Set-Content -Path $settingsPath -Encoding UTF8
    Write-Success "  已配置 ANTHROPIC_BASE_URL=$RouterBaseUrl"
}

function Update-UserEnvironment {
    Write-Info '[修复] 写入用户环境变量...'
    [Environment]::SetEnvironmentVariable('ANTHROPIC_BASE_URL', $RouterBaseUrl, 'User')
    $env:ANTHROPIC_BASE_URL = $RouterBaseUrl

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $paths = @($userPath -split ';' | Where-Object { $_ })
    if ($paths -notcontains $InstallDir) {
        $newUserPath = if ($userPath) { "$InstallDir;$userPath" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable('Path', $newUserPath, 'User')
    }
    $env:Path = "$InstallDir;$env:Path"
    Write-Success '  用户环境变量已更新。'
}

function Start-MtlsRouter {
    Write-Info '[启动] 启动 mtls-router 后台模式...'
    & $BinaryPath -backend
    if ($LASTEXITCODE -ne 0) { Write-Fail "mtls-router 启动失败，退出码：$LASTEXITCODE" }
    Write-Success "  mtls-router 已启动，监听地址通常为 $RouterBaseUrl"
}

function Main {
    Show-Banner
    Download-MtlsRouter
    Ensure-ClaudeCode
    Update-ClaudeSettings
    Update-UserEnvironment
    Start-MtlsRouter

    Write-Success '============================================================'
    Write-Success '配置完成！即将启动 Claude Code。'
    Write-Success '============================================================'
    claude
}

Main
