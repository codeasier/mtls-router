# internal/proxy

反向代理核心：构建带 mTLS transport、SSE 流式与脱敏错误处理的 `httputil.ReverseProxy`。

## 文件

| 文件 | 职责 |
|------|------|
| `proxy.go` | `New(Options)` -- 装配 ReverseProxy，含 director、ModifyResponse、ErrorHandler、`FlushInterval: -1` |
| `transport.go` | `NewMTLSTransport(certPEM, keyPEM, caPEM, ...TransportOption)` -- 构建内嵌 mTLS 证书的 `*http.Transport` |
| `director.go` | `NewDirector(upstream)` -- 仅改写请求 Scheme/Host；保留 path/query/body |
| `modifyresponse.go` | `NewModifyResponse()` -- 规范化 SSE 头（`text/event-stream`、`no-cache`、`X-Accel-Buffering: no`） |
| `errorhandler.go` | `NewErrorHandler(logger)` -- 返回脱敏 JSON 错误；分类 502/504/400 且不泄露 upstream 细节 |
| `bodyerror.go` | `wrapBodyReadErrors(r)` -- 包装请求体，将客户端读取失败标记为 `clientBodyReadError` |

## 关键不变量

- `FlushInterval: -1` 对 SSE/chunked 流式是必需的 -- 绝不缓冲响应。
- 错误 JSON 绝不可含 upstream URL、证书细节或原始错误字符串；测试会断言脱敏。
- `sanitizedProxyLogWriter` 替换默认 `log.Logger` 输出，抑制 `httputil` 内部消息。
- 不要在此添加请求管线中间件；hook 应在 `main.go` 的 mux 调用点组合。

## 依赖

- `internal/certs` -- PEM 字符串 -> `tls.Certificate` + `x509.CertPool`
- `internal/tlspolicy` -- 版本字符串 -> `uint16` TLS 常量

## 测试

- `service_contract_test.go` -- 端到端代理契约断言
- `image_contract_test.go` -- 图片路径契约：`GET /v1/models/image`、`POST /v1/images/generations` 生图与 data URI 编辑、大型 `b64_json` 响应不缓冲、downstream 取消、upstream 失败脱敏、访问日志不泄露 key/prompt/base64
- `image_fixture_test.go` -- 9Router fixture schema/来源一致性测试（`testdata/9router-image/`），确保模型 ID 漂移 fail closed
- `bodyerror_policy_test.go` -- 客户端 body 错误 -> 400 分类
- `transport_test.go` -- mTLS transport 构造与 TLS 最低版本强制
