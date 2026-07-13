FROM golang:1.26.2-alpine AS build

WORKDIR /src
RUN apk add --no-cache openssl
COPY go.mod ./
COPY . .
ARG VERSION=dev
ARG COMMIT=unknown
ARG BUILD_DATE=unknown
ARG DEPLOYMENT_ID=dev
RUN mkdir -p /tmp/certs /out \
    && openssl req -x509 -newkey rsa:2048 -nodes -keyout /tmp/certs/client.key -out /tmp/certs/client.crt -subj "/CN=mtls-router-client" -days 1 \
    && openssl req -x509 -newkey rsa:2048 -nodes -keyout /tmp/certs/upstream-ca.key -out /tmp/certs/upstream-ca.crt -subj "/CN=mtls-router-upstream-ca" -days 1 \
    && CGO_ENABLED=0 go build -ldflags "-s -w -X 'main.clientCertPEM=$(cat /tmp/certs/client.crt)' -X 'main.clientKeyPEM=$(cat /tmp/certs/client.key)' -X 'main.upstreamCAPEM=$(cat /tmp/certs/upstream-ca.crt)' -X 'main.upstreamURL=https://example.invalid' -X 'github.com/codeasier/mtls-router/internal/version.Version=${VERSION}' -X 'github.com/codeasier/mtls-router/internal/version.Commit=${COMMIT}' -X 'github.com/codeasier/mtls-router/internal/version.BuildDate=${BUILD_DATE}' -X 'github.com/codeasier/mtls-router/internal/version.DeploymentID=${DEPLOYMENT_ID}'" -o /out/mtls-router .

FROM scratch

COPY --from=build /out/mtls-router /mtls-router
EXPOSE 19099
USER 65534
ENTRYPOINT ["/mtls-router"]
