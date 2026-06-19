# 构建与发布

[English](../BUILD.md)

本文档面向需要从源码构建 `mtls-router` 或发布 GitHub Release 二进制文件的维护者。

## 本地占位构建

本地开发时运行：

```bash
./scripts/build.sh
```

如果以下文件不存在，脚本会自动创建：

- `secrets/client.pem`
- `secrets/client.key`
- `secrets/upstream-ca.pem`

随后脚本会执行 `go build -trimpath`，并输出 `./mtls-router`。

生成的占位二进制预期会在启动时快速失败，直到使用真实的上游配置和证书材料重新构建。

## 使用真实证书构建

```bash
go build -trimpath \
  -ldflags "-s -w \
    -X 'main.clientCertPEM=$(cat secrets/client.pem)' \
    -X 'main.clientKeyPEM=$(cat secrets/client.key)' \
    -X 'main.upstreamCAPEM=$(cat secrets/upstream-ca.pem)' \
    -X 'main.upstreamURL=https://router.example.com'" \
  -o mtls-router .
```

二进制文件不会在运行时读取证书文件。证书 PEM、私钥 PEM、上游 CA PEM 和默认上游 URL 都会通过 linker variables 在构建期嵌入：

- `main.clientCertPEM`
- `main.clientKeyPEM`
- `main.upstreamCAPEM`
- `main.upstreamURL`
- `main.version`

## GitHub Release 配置

release workflow 会读取以下 repository secrets：

- `CLIENT_CERT_PEM`
- `CLIENT_KEY_PEM`
- `UPSTREAM_CA_PEM`

它也会读取以下 repository variable：

- `UPSTREAM_URL`

使用 `gh` 设置：

```bash
gh secret set CLIENT_CERT_PEM --repo codeasier/mtls-router < secrets/client.pem
gh secret set CLIENT_KEY_PEM --repo codeasier/mtls-router < secrets/client.key
gh secret set UPSTREAM_CA_PEM --repo codeasier/mtls-router < secrets/upstream-ca.pem
gh variable set UPSTREAM_URL --repo codeasier/mtls-router --body "https://router.example.com"
```

## 发布版本

推送版本 tag：

```bash
git tag v0.1.1
git push origin v0.1.1
```

GitHub Actions release workflow 会为以下平台交叉编译二进制文件：

- `linux/amd64`
- `linux/arm64`
- `darwin/amd64`
- `darwin/arm64`
- `windows/amd64`
- `windows/arm64`

workflow 会将每个二进制文件上传为 workflow artifact；对于 tag 构建，还会把这些二进制文件附加到 GitHub Release。
