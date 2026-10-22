//go:build unix

package routerenroll

import (
	"bufio"
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
)

func freeLoopback(t *testing.T) string {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	return l.Addr().String()
}

func TestRealNginxRenewalAndObservedCRLLoad(t *testing.T) {
	nginx, err := exec.LookPath("nginx")
	if err != nil {
		t.Skip("nginx executable required for real TLS/CRL integration")
	}
	f := newRenewalFixture(t)
	// A real-clock, one-hour predecessor under the same test authority.
	f.now = time.Now().UTC().Truncate(time.Second)
	oldTemplate := *f.parsed
	oldTemplate.NotAfter = f.now.Add(time.Hour)
	der, err := x509.CreateCertificate(rand.Reader, &oldTemplate, f.s.issuer.cert, &f.key.PublicKey, f.s.issuer.key)
	if err != nil {
		t.Fatal(err)
	}
	f.old.CertificatePEM = string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}))
	f.old.NotAfter = oldTemplate.NotAfter.Unix()
	f.parsed, _ = x509.ParseCertificate(der)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if err := f.s.store.transaction(ctx, func(tx *writeTx) error {
		_, err := tx.ExecContext(ctx, "UPDATE certificates SET certificate_pem=?,not_after=? WHERE serial=?", f.old.CertificatePEM, f.old.NotAfter, f.old.Serial)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	dir, err := os.MkdirTemp("", "enroll-nginx-")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(dir)
	write := func(name string, data []byte) string {
		t.Helper()
		p := filepath.Join(dir, name)
		if os.WriteFile(p, data, 0600) != nil {
			t.Fatal("write isolated nginx fixture")
		}
		return p
	}
	serverRoot := newFixtureCA(t, nil, 90, f.now)
	serverKey, _ := keyedCSR(t)
	serverDER, err := x509.CreateCertificate(rand.Reader, &x509.Certificate{SerialNumber: big.NewInt(91), NotBefore: f.now.Add(-ClockSkew), NotAfter: f.now.Add(time.Hour), DNSNames: []string{"relay.codeasier.top", "enroll.codeasier.top"}, KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}, serverRoot.cert, &serverKey.PublicKey, serverRoot.key)
	if err != nil {
		t.Fatal(err)
	}
	serverCertFile := write("server.pem", pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: serverDER}))
	serverKeyFile := write("server.key", privateKeyPEM(t, serverKey))
	// The root key exists only in this in-memory synthetic fixture.
	// fixtureIssuer does not retain the offline key; regenerate a complete test
	// hierarchy and re-sign the existing predecessor under that hierarchy.
	issuer, issuerCA, rootCA := fixtureIssuer(t, f.now)
	f.s.issuer = issuer
	oldTemplate.Raw = nil
	oldTemplate.RawTBSCertificate = nil
	oldTemplate.RawIssuer = nil
	der, err = x509.CreateCertificate(rand.Reader, &oldTemplate, issuerCA.cert, &f.key.PublicKey, issuerCA.key)
	if err != nil {
		t.Fatal(err)
	}
	oldIssuer := f.old.IssuerID
	f.old.IssuerID = issuer.id
	f.old.ChainPEM = issuer.chain
	f.old.CertificatePEM = string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}))
	f.parsed, _ = x509.ParseCertificate(der)
	f.request.IssuerID = issuer.id
	if err := f.s.store.transaction(ctx, func(tx *writeTx) error {
		_, err := tx.ExecContext(ctx, "UPDATE certificates SET issuer_id=?,certificate_pem=?,chain_pem=? WHERE issuer_id=? AND serial=?", issuer.id, f.old.CertificatePEM, issuer.chain, oldIssuer, f.old.Serial)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	rootDER := fixtureRootCRL(t, rootCA, 1, f.now)
	pub, err := f.s.store.BuildCRL(ctx, issuer, rootDER, f.now)
	if err != nil {
		t.Fatal(err)
	}
	bundle := GatewayBundle{pub, rootDER}
	gw := GatewayLoadConfig{ID: "jump", StateDir: filepath.Join(dir, "gateway"), IssuerCertFile: write("issuing.pem", certificatePEM(issuer.cert)), RootCAFile: write("root.pem", certificatePEM(issuer.root))}
	if _, err := ApplyGatewayBundle(ctx, gw, bundle, &fakeGatewayRuntime{marker: gatewayMarker(bundle)}, f.now); err != nil {
		t.Fatal(err)
	}
	publicAddress, relayAddress, enrollAddress, probeAddress := freeLoopback(t), freeLoopback(t), freeLoopback(t), freeLoopback(t)
	socket := filepath.Join(dir, "renew.sock")
	public, _ := NewHTTPServer(publicAddress, f.s.Handler(), nil)
	controlDone := make(chan error, 1)
	go func() { controlDone <- ServeControl(ctx, public, socket, f.s.GatewayHandler(), nil) }()
	defer func() {
		cancel()
		select {
		case <-controlDone:
		case <-time.After(6 * time.Second):
			t.Error("control server did not stop")
		}
	}()
	var backendCalls atomic.Int32
	uploadStarted := make(chan struct{})
	cancelObserved := make(chan struct{})
	streamRelease := make(chan struct{})
	defer func() {
		select {
		case <-streamRelease:
		default:
			close(streamRelease)
		}
	}()
	backendHandler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		backendCalls.Add(1)
		if r.Header.Get(GatewayCertificateHeader) != "" || r.Header.Get(GatewayVerifyHeader) != "" {
			t.Error("reserved identity heads leaked")
		}
		if r.Header.Get("Authorization") != "Bearer business-canary" {
			t.Error("business authorization changed")
		}
		if strings.HasPrefix(r.URL.Path, "/v1/status/") {
			status, _ := strconv.Atoi(strings.TrimPrefix(r.URL.Path, "/v1/status/"))
			w.WriteHeader(status)
			return
		}
		if r.URL.Path == "/v1/upload" {
			var part [4]byte
			if _, err := io.ReadFull(r.Body, part[:]); err == nil {
				close(uploadStarted)
			}
			_, _ = io.Copy(io.Discard, r.Body)
			w.WriteHeader(204)
			return
		}
		if r.URL.Path == "/v1/cancel" {
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = io.WriteString(w, "data: first\n\n")
			w.(http.Flusher).Flush()
			<-r.Context().Done()
			close(cancelObserved)
			return
		}
		if r.URL.Path == "/v1/bench-stream" {
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = io.WriteString(w, "data: first\n\n")
			w.(http.Flusher).Flush()
			<-r.Context().Done()
			return
		}
		if r.URL.Path == "/v1/stream" {
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = io.WriteString(w, "data: first\n\n")
			w.(http.Flusher).Flush()
			select {
			case <-streamRelease:
				_, _ = io.WriteString(w, "data: done\n\n")
			case <-r.Context().Done():
			}
			return
		}
		_, _ = io.WriteString(w, "model-ok")
	})
	backend := httptest.NewServer(backendHandler)
	defer backend.Close()
	backend2 := httptest.NewServer(backendHandler)
	defer backend2.Close()
	userDirective := ""
	if os.Geteuid() == 0 {
		userDirective = "user root;"
	}
	template, err := os.ReadFile("../../deploy/nginx/codeasier-personal.conf.example")
	if err != nil {
		t.Fatal(err)
	}
	adapted := strings.NewReplacer(
		"/etc/nginx/tls/enroll-fullchain.pem", serverCertFile, "/etc/nginx/tls/relay-fullchain.pem", serverCertFile,
		"/etc/nginx/tls/enroll-key.pem", serverKeyFile, "/etc/nginx/tls/relay-key.pem", serverKeyFile,
		"/etc/codeasier-router-gateway/root-cert.pem", gw.RootCAFile, "/var/lib/codeasier-router-gateway", gw.StateDir,
		"/run/codeasier-router-enroll/gateway.sock", socket, "127.0.0.1:19101", publicAddress, "127.0.0.1:19102", probeAddress,
		"REPLACE_WITH_VERIFIED_BACKEND:20128", strings.TrimPrefix(backend.URL, "http://")+"; server "+strings.TrimPrefix(backend2.URL, "http://"), "/var/log/nginx/", dir+"/",
	).Replace(string(template))
	adapted = strings.Replace(adapted, "listen 443 ssl;", "listen "+enrollAddress+" ssl;", 1)
	adapted = strings.Replace(adapted, "listen 443 ssl;", "listen "+relayAddress+" ssl;", 1)
	configuration := fmt.Sprintf("%s\nworker_processes 1; pid \"%s/nginx.pid\"; error_log \"%s/error.log\" crit; events {worker_connections 128;} http {\n%s\n}\n", userDirective, dir, dir, adapted)
	configFile := write("nginx.conf", []byte(configuration))
	validation := exec.Command(nginx, "-p", dir, "-c", configFile, "-t")
	if data, err := validation.CombinedOutput(); err != nil {
		t.Fatalf("isolated nginx config failed: %s", data)
	}
	cmd := exec.Command(nginx, "-p", dir, "-c", configFile, "-g", "daemon off;")
	cmd.Stdout = io.Discard
	cmd.Stderr = io.Discard
	if cmd.Start() != nil {
		t.Fatal("isolated nginx failed to start")
	}
	nginxDone := make(chan error, 1)
	go func() { nginxDone <- cmd.Wait() }()
	defer func() {
		_ = cmd.Process.Signal(syscall.SIGQUIT)
		select {
		case <-nginxDone:
		case <-time.After(5 * time.Second):
			_ = cmd.Process.Kill()
			<-nginxDone
		}
	}()
	runtime := NginxRuntime{Binary: nginx, PidFile: filepath.Join(dir, "nginx.pid"), prefix: dir, config: configFile, probeURL: "http://" + probeAddress + "/crl-loaded"}
	deadline := time.Now().Add(5 * time.Second)
	for {
		if marker, err := runtime.Marker(ctx); err == nil && marker == gatewayMarker(bundle) {
			break
		}
		if time.Now().After(deadline) {
			data, _ := os.ReadFile(filepath.Join(dir, "error.log"))
			t.Fatalf("nginx did not become ready: %s", data)
		}
		time.Sleep(10 * time.Millisecond)
	}
	if _, err := ApplyGatewayBundle(ctx, gw, bundle, runtime, f.now); err != nil {
		t.Fatal("real generation not acknowledged", err)
	}
	gw2 := gw
	gw2.ID = "jump2"
	gw2.StateDir = filepath.Join(dir, "gateway-second")
	if _, err := ApplyGatewayBundle(ctx, gw2, bundle, &fakeGatewayRuntime{marker: gatewayMarker(bundle)}, f.now); err != nil {
		t.Fatal(err)
	}
	relay2, enroll2, probe2 := freeLoopback(t), freeLoopback(t), freeLoopback(t)
	secondAdapted := strings.NewReplacer(gw.StateDir, gw2.StateDir, relayAddress, relay2, enrollAddress, enroll2, probeAddress, probe2).Replace(adapted)
	secondConfiguration := fmt.Sprintf("%s\nworker_processes 1; pid \"%s/nginx-second.pid\"; error_log \"%s/error-second.log\" crit; events {worker_connections 128;} http {\n%s\n}\n", userDirective, dir, dir, secondAdapted)
	secondFile := write("nginx-second.conf", []byte(secondConfiguration))
	secondCommand := exec.Command(nginx, "-p", dir, "-c", secondFile, "-g", "daemon off;")
	secondCommand.Stdout = io.Discard
	secondCommand.Stderr = io.Discard
	if secondCommand.Start() != nil {
		t.Fatal("second isolated node failed")
	}
	secondDone := make(chan error, 1)
	go func() { secondDone <- secondCommand.Wait() }()
	defer func() {
		_ = secondCommand.Process.Signal(syscall.SIGQUIT)
		select {
		case <-secondDone:
		case <-time.After(5 * time.Second):
			_ = secondCommand.Process.Kill()
			<-secondDone
		}
	}()
	runtime2 := NginxRuntime{Binary: nginx, PidFile: filepath.Join(dir, "nginx-second.pid"), prefix: dir, config: secondFile, probeURL: "http://" + probe2 + "/crl-loaded"}
	secondDeadline := time.Now().Add(5 * time.Second)
	for {
		if _, err := runtime2.Marker(ctx); err == nil {
			break
		}
		if time.Now().After(secondDeadline) {
			t.Fatal("second node not ready")
		}
		time.Sleep(10 * time.Millisecond)
	}
	if _, err := ApplyGatewayBundle(ctx, gw2, bundle, runtime2, f.now); err != nil {
		t.Fatal(err)
	}
	exerciseNginxReloadRetirement(t, runtime2)
	clientFor := func(address string, certificate *tls.Certificate) *http.Client {
		roots := x509.NewCertPool()
		roots.AddCert(serverRoot.cert)
		tlsConfig := &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12, ClientSessionCache: tls.NewLRUClientSessionCache(8)}
		if certificate != nil {
			tlsConfig.Certificates = []tls.Certificate{*certificate}
		}
		transport := &http.Transport{TLSClientConfig: tlsConfig, DisableKeepAlives: true, ForceAttemptHTTP2: false, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			return (&net.Dialer{Timeout: time.Second}).DialContext(ctx, "tcp", address)
		}}
		t.Cleanup(transport.CloseIdleConnections)
		return &http.Client{Transport: transport, Timeout: 3 * time.Second}
	}
	pair, err := tls.X509KeyPair([]byte(f.old.CertificatePEM+issuer.chain), privateKeyPEM(t, f.key))
	if err != nil {
		t.Fatal(err)
	}
	client := clientFor(relayAddress, &pair)
	client2 := clientFor(relay2, &pair)
	model := func(client *http.Client) int {
		t.Helper()
		r, _ := http.NewRequest("POST", RelayOrigin+"/v1/models", strings.NewReader("stream-canary"))
		r.Header.Set("Authorization", "Bearer business-canary")
		r.Header.Set(GatewayVerifyHeader, "attacker")
		r.Header.Set(GatewayCertificateHeader, "attacker")
		response, err := client.Do(r)
		if err != nil {
			return 0
		}
		defer response.Body.Close()
		_, _ = io.Copy(io.Discard, response.Body)
		return response.StatusCode
	}
	if status := model(client); status != 200 {
		t.Fatal("valid mTLS request rejected", status)
	}
	if status := model(client2); status != 200 {
		t.Fatal("same device certificate failed on second node", status)
	}
	beforeWrongName := backendCalls.Load()
	wrongName, _ := http.NewRequest("GET", "https://wrong-host.invalid/v1/models", nil)
	wrongName.Header.Set("Authorization", "Bearer business-canary")
	if response, err := client.Do(wrongName); err == nil {
		_ = response.Body.Close()
		t.Fatal("hostname verification was bypassed")
	}
	if backendCalls.Load() != beforeWrongName {
		t.Fatal("wrong server name received a business request")
	}
	if status := model(clientFor(relayAddress, nil)); status == 200 {
		t.Fatal("no-certificate request forwarded")
	}
	for _, kind := range []string{"old_shared_ca", "server_eku", "expired"} {
		parent, key := issuer.cert, issuer.key
		leaf := *f.parsed
		leaf.SerialNumber = big.NewInt(100)
		switch kind {
		case "old_shared_ca":
			legacy := newFixtureCA(t, nil, 101, f.now)
			parent, key = legacy.cert, legacy.key
		case "server_eku":
			leaf.ExtKeyUsage = []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}
		case "expired":
			leaf.NotBefore = f.now.Add(-2 * time.Hour)
			leaf.NotAfter = f.now.Add(-time.Hour)
		}
		der, err := x509.CreateCertificate(rand.Reader, &leaf, parent, &f.key.PublicKey, key)
		if err != nil {
			t.Fatal(err)
		}
		bad, err := tls.X509KeyPair(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), privateKeyPEM(t, f.key))
		if err != nil {
			t.Fatal(err)
		}
		bad.Certificate = append(bad.Certificate, parent.Raw)
		before := backendCalls.Load()
		if status := model(clientFor(relayAddress, &bad)); status == 200 || backendCalls.Load() != before {
			t.Fatal("invalid client identity forwarded", kind, status)
		}
	}
	if f.calls.Load() != 0 {
		t.Fatal("model requests queried control-plane provider")
	}
	for _, status := range []int{401, 403, 429, 500, 502, 503} {
		before := backendCalls.Load()
		r, _ := http.NewRequest("POST", RelayOrigin+"/v1/status/"+strconv.Itoa(status), nil)
		r.Header.Set("Authorization", "Bearer business-canary")
		response, err := client.Do(r)
		if err != nil {
			t.Fatal(err)
		}
		_, _ = io.Copy(io.Discard, response.Body)
		_ = response.Body.Close()
		if response.StatusCode != status || backendCalls.Load() != before+1 {
			t.Fatal("business status changed or request replayed", status, response.StatusCode)
		}
	}
	uploadRead, uploadWrite := io.Pipe()
	defer uploadRead.Close()
	defer uploadWrite.Close()
	uploadRequest, _ := http.NewRequest("POST", RelayOrigin+"/v1/upload", uploadRead)
	uploadRequest.Header.Set("Authorization", "Bearer business-canary")
	uploadDone := make(chan error, 1)
	go func() {
		response, err := client.Do(uploadRequest)
		if err == nil {
			_, _ = io.Copy(io.Discard, response.Body)
			_ = response.Body.Close()
			if response.StatusCode != 204 {
				err = fmt.Errorf("upload status")
			}
		}
		uploadDone <- err
	}()
	firstWritten := make(chan struct{})
	go func() { _, _ = uploadWrite.Write([]byte("part")); close(firstWritten) }()
	select {
	case <-uploadStarted:
	case <-time.After(time.Second):
		t.Fatal("nginx buffered upload until EOF")
	}
	<-firstWritten
	go func() { _, err := uploadWrite.Write(make([]byte, 2*1024*1024)); _ = uploadWrite.CloseWithError(err) }()
	if err := <-uploadDone; err != nil {
		t.Fatal("large streaming upload failed", err)
	}
	cancelContext, stopRequest := context.WithCancel(ctx)
	defer stopRequest()
	cancelRequest, _ := http.NewRequestWithContext(cancelContext, "GET", RelayOrigin+"/v1/cancel", nil)
	cancelRequest.Header.Set("Authorization", "Bearer business-canary")
	cancelled, err := client.Do(cancelRequest)
	if err != nil {
		t.Fatal(err)
	}
	stopRequest()
	_ = cancelled.Body.Close()
	select {
	case <-cancelObserved:
	case <-time.After(time.Second):
		t.Fatal("cancellation did not reach backend")
	}
	if f.calls.Load() != 0 {
		t.Fatal("business status/upload/cancellation consulted control plane")
	}
	benchmarkIsolatedGateway(t, client, cmd.Process.Pid, func() error { _, err := ApplyGatewayBundle(ctx, gw, bundle, runtime, f.now); return err })
	if f.calls.Load() != 0 {
		t.Fatal("load probe consulted control plane")
	}
	nativePublic := clientFor(enrollAddress, nil)
	nativePublic.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	nativeCertificate, nativeKey := exerciseNativeThroughNginx(t, f, nativePublic, true)
	nativePair, err := tls.X509KeyPair([]byte(nativeCertificate.CertificatePEM+nativeCertificate.ChainPEM), privateKeyPEM(t, nativeKey))
	if err != nil {
		t.Fatal("native result not bound to device key")
	}
	beforeNativeModel := f.calls.Load()
	if status := model(clientFor(relay2, &nativePair)); status != 200 || f.calls.Load() != beforeNativeModel {
		t.Fatal("fresh native identity failed or invoked online control", status)
	}
	_, _ = exerciseNativeThroughNginx(t, f, nativePublic, false)
	if os.Getenv("MTLS_TEST_RUST_NATIVE") == "1" {
		serverRootFile := write("server-root.pem", certificatePEM(serverRoot.cert))
		_, enrollPort, _ := net.SplitHostPort(enrollAddress)
		_, relayPort, _ := net.SplitHostPort(relayAddress)
		command := exec.Command("cargo", "test", "--locked", "--offline", "-p", "router-identity", "tests::external_https_enrollment", "--", "--ignored", "--exact", "--nocapture")
		command.Dir = filepath.Join("..", "..")
		command.Env = append(os.Environ(), "MTLS_TEST_NATIVE_CLIENT_ROOT="+gw.RootCAFile, "MTLS_TEST_NATIVE_SERVER_ROOT="+serverRootFile, "MTLS_TEST_NATIVE_ENROLL="+EnrollOrigin+":"+enrollPort, "MTLS_TEST_NATIVE_RELAY="+RelayOrigin+":"+relayPort)
		if output, err := command.CombinedOutput(); err != nil {
			t.Fatalf("real Rust native enrollment failed: %s", output)
		}
		t.Log("actual Rust NativeClient completed HTTPS enrollment, mTLS model call and device logout")
	}
	transport := client.Transport.(*http.Transport)
	for range 2 {
		r, _ := http.NewRequest("GET", RelayOrigin+"/v1/models", nil)
		r.Header.Set("Authorization", "Bearer business-canary")
		res, err := client.Do(r)
		if err != nil {
			t.Fatal(err)
		}
		_, _ = io.Copy(io.Discard, res.Body)
		_ = res.Body.Close()
		if res.TLS.DidResume {
			t.Fatal("TLS session resumed despite revocation baseline")
		}
	}
	rawConfig := transport.TLSClientConfig.Clone()
	rawConfig.ServerName = "relay.codeasier.top"
	rawConfig.NextProtos = []string{"h2", "http/1.1"}
	conn, err := tls.DialWithDialer(&net.Dialer{Timeout: time.Second}, "tcp", relayAddress, rawConfig)
	if err != nil {
		t.Fatal(err)
	}
	if conn.ConnectionState().NegotiatedProtocol == "h2" {
		_ = conn.Close()
		t.Fatal("HTTP/2 multiplexing enabled")
	}
	_ = conn.SetDeadline(time.Now().Add(2 * time.Second))
	rawRequest, _ := http.NewRequest("GET", RelayOrigin+"/v1/models", nil)
	rawRequest.Header.Set("Authorization", "Bearer business-canary")
	reader := bufio.NewReader(conn)
	beforeReuse := backendCalls.Load()
	if rawRequest.Write(conn) != nil {
		t.Fatal("first raw request")
	}
	first, err := http.ReadResponse(reader, rawRequest)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = io.Copy(io.Discard, first.Body)
	_ = first.Body.Close()
	if rawRequest.Write(conn) == nil {
		if reused, err := http.ReadResponse(reader, rawRequest); err == nil {
			_ = reused.Body.Close()
			_ = conn.Close()
			t.Fatal("second request reused authenticated connection")
		}
	}
	_ = conn.Close()
	if backendCalls.Load() != beforeReuse+1 {
		t.Fatal("keepalive bypass reached backend")
	}
	requestBody, _ := json.Marshal(f.sign(t, false))
	r, _ := http.NewRequest("POST", RelayOrigin+RenewPath, bytes.NewReader(requestBody))
	r.Header.Set("Content-Type", "application/json")
	response, err := client.Do(r)
	if err != nil {
		t.Fatal("real mTLS renewal failed", err)
	}
	var next IssuedCertificate
	decodeErr := json.NewDecoder(response.Body).Decode(&next)
	_ = response.Body.Close()
	if response.StatusCode != 200 || decodeErr != nil || next.Serial == f.old.Serial || next.Serial == "" {
		t.Fatal("trusted nginx renewal headers not accepted", response.StatusCode, decodeErr)
	}
	streamRequest, _ := http.NewRequest("GET", RelayOrigin+"/v1/stream", nil)
	streamRequest.Header.Set("Authorization", "Bearer business-canary")
	streamClient := *client
	streamClient.Timeout = 10 * time.Second
	stream, err := streamClient.Do(streamRequest)
	if err != nil {
		t.Fatal(err)
	}
	defer stream.Body.Close()
	streamReader := bufio.NewReader(stream.Body)
	if line, err := streamReader.ReadString('\n'); err != nil || line != "data: first\n" {
		t.Fatal("SSE first event buffered", err)
	}
	idle, err := tls.DialWithDialer(&net.Dialer{Timeout: time.Second}, "tcp", relayAddress, rawConfig)
	if err != nil {
		t.Fatal(err)
	}
	defer idle.Close()
	_ = idle.SetDeadline(time.Now().Add(8 * time.Second))
	if _, err := revoke(ctx, f.s.store, RevocationTarget{"serial", f.old.Serial}, f.now); err != nil {
		t.Fatal(err)
	}
	nextPub, err := f.s.store.BuildCRL(ctx, issuer, rootDER, f.now)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ApplyGatewayBundle(ctx, gw, GatewayBundle{nextPub, rootDER}, runtime, f.now); err != nil {
		t.Fatal("actual revoked CRL load not observed", err)
	}
	if status := model(client2); status != 200 {
		t.Fatal("unacknowledged node did not preserve its previous isolated CRL state", status)
	}
	if _, err := ApplyGatewayBundle(ctx, gw2, GatewayBundle{nextPub, rootDER}, runtime2, f.now); err != nil {
		t.Fatal(err)
	}
	before := backendCalls.Load()
	if status := model(client); status == 200 || backendCalls.Load() != before {
		t.Fatal("revoked serial reached business backend", status)
	}
	if status := model(client2); status != 403 || backendCalls.Load() != before {
		t.Fatal("second node still admitted revoked identity", status)
	}
	close(streamRelease)
	remainder, err := io.ReadAll(streamReader)
	_ = stream.Body.Close()
	if err != nil || !strings.Contains(string(remainder), "data: done") {
		t.Fatal("in-flight SSE interrupted by CRL reload", err)
	}
	time.Sleep(5100 * time.Millisecond)
	before = backendCalls.Load()
	if rawRequest.Write(idle) == nil {
		if response, err := http.ReadResponse(bufio.NewReader(idle), rawRequest); err == nil {
			_ = response.Body.Close()
			t.Fatal("pre-reload idle connection outlived bounded header window")
		}
	}
	if backendCalls.Load() != before {
		t.Fatal("old idle connection reached backend after window")
	}
	siblingKey, siblingCSR := keyedCSR(t)
	sibling, err := issuer.Issue(siblingCSR, fixtureIdentity(), f.now)
	if err != nil {
		t.Fatal(err)
	}
	siblingPair, err := tls.X509KeyPair([]byte(sibling.CertificatePEM+issuer.chain), privateKeyPEM(t, siblingKey))
	if err != nil {
		t.Fatal(err)
	}
	siblingClient := clientFor(relayAddress, &siblingPair)
	if status := model(siblingClient); status != 200 {
		t.Fatal("unrevoked sibling was rejected", status)
	}
	siblingClient2 := clientFor(relay2, &siblingPair)
	if status := model(siblingClient2); status != 200 {
		t.Fatal("unrevoked sibling failed on second node", status)
	}
	revokedRoot := fixtureRootCRL(t, rootCA, 2, f.now, issuer.cert)
	if _, err := ApplyGatewayBundle(ctx, gw, GatewayBundle{nextPub, revokedRoot}, runtime, f.now); err != nil {
		t.Fatal("valid root revocation was not loadable", err)
	}
	before = backendCalls.Load()
	if status := model(siblingClient); status != 403 || backendCalls.Load() != before {
		t.Fatal("revoked issuer still admitted an unlisted leaf", status)
	}
	if _, err := ApplyGatewayBundle(ctx, gw2, GatewayBundle{nextPub, revokedRoot}, runtime2, f.now); err != nil {
		t.Fatal(err)
	}
	if status := model(siblingClient2); status != 403 || backendCalls.Load() != before {
		t.Fatal("second node ignored root authority revocation", status)
	}
	t.Log("real nginx: strict mTLS, trusted Unix renewal, original API key and observed reload/CRL refusal passed")
}

func exerciseNativeThroughNginx(t *testing.T, f *renewalFixture, client *http.Client, member bool) (IssuedCertificate, *ecdsa.PrivateKey) {
	t.Helper()
	if !member {
		original := f.s.provider.client.Transport
		f.s.provider.client.Transport = providerTransport(func(r *http.Request) (*http.Response, error) {
			body := ""
			switch r.URL.Scheme + "://" + r.URL.Host + r.URL.Path {
			case userURL:
				body = `{"id":"nonmember-fixture","login":"nonmember"}`
			case membershipURL + "msinsight-org":
				body = `{"active":false,"user":{"id":"nonmember-fixture"},"organization":{"id":"1783195","login":"msinsight-org"}}`
			default:
				return original.RoundTrip(r)
			}
			return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(body)), Request: r}, nil
		})
		defer func() { f.s.provider.client.Transport = original }()
	}
	request := func(method, path, contentType string, body io.Reader, cookie *http.Cookie) (int, []byte, http.Header) {
		t.Helper()
		r, err := http.NewRequest(method, EnrollOrigin+path, body)
		if err != nil {
			t.Fatal(err)
		}
		if contentType != "" {
			r.Header.Set("Content-Type", contentType)
		}
		if method == "POST" && contentType == "application/x-www-form-urlencoded" {
			r.Header.Set("Origin", EnrollOrigin)
		}
		if cookie != nil {
			r.AddCookie(cookie)
		}
		response, err := client.Do(r)
		if err != nil {
			t.Fatal("native HTTPS request failed", err)
		}
		data, err := io.ReadAll(response.Body)
		_ = response.Body.Close()
		if err != nil {
			t.Fatal(err)
		}
		return response.StatusCode, data, response.Header
	}
	postJSON := func(path string, value any) (int, []byte) {
		data, _ := json.Marshal(value)
		status, body, _ := request("POST", path, "application/json", bytes.NewReader(data), nil)
		return status, body
	}
	key, csr := keyedCSR(t)
	verifier, _ := randomProof()
	device := "b5c2fb58-d5e8-4dc2-9b04-38209df797d8"
	if !member {
		device = "b5c2fb58-d5e8-4dc2-9b04-38209df797d9"
	}
	start := NativeStart{Version: ProtocolVersion, CSRPEM: csr, DeviceID: device, DeviceKind: "linux_cli", DeviceLabel: "native nginx fixture", Challenge: Challenge(verifier), ChallengeMethod: "S256"}
	status, body := postJSON("/enroll/native/start", start)
	var started NativeStarted
	if status != 201 || json.Unmarshal(body, &started) != nil {
		t.Fatal("native start", status)
	}
	path := "/enroll/native/verify?transaction=" + url.QueryEscape(started.Transaction)
	status, page, headers := request("GET", path, "", nil, nil)
	if status != 200 {
		t.Fatal("browser verification", status)
	}
	response := http.Response{Header: headers}
	cookies := response.Cookies()
	if len(cookies) != 1 || !cookies[0].Secure || !cookies[0].HttpOnly {
		t.Fatal("browser binding cookie")
	}
	cookie := cookies[0]
	authorize := url.Values{"transaction": {started.Transaction}, "csrf": {extractCSRF(t, string(page))}}
	status, _, headers = request("POST", "/enroll/native/authorize", "application/x-www-form-urlencoded", strings.NewReader(authorize.Encode()), cookie)
	if status != 303 {
		t.Fatal("native OAuth authorization", status)
	}
	location, err := url.Parse(headers.Get("Location"))
	if err != nil || location.Scheme+"://"+location.Host+location.Path != authorizeURL {
		t.Fatal("OAuth origin override")
	}
	callback := "/enroll/gitcode/callback?" + url.Values{"state": {location.Query().Get("state")}, "code": {"fixture-oauth-code"}}.Encode()
	status, page, _ = request("GET", callback, "", nil, cookie)
	proof := NativeProof{Transaction: started.Transaction, Verifier: verifier}
	if !member {
		if status != 403 {
			t.Fatal("nonmember callback accepted", status)
		}
		status, body = postJSON("/enroll/native/poll", proof)
		if status != 403 || !bytes.Contains(body, []byte(MembershipRequired)) {
			t.Fatal("nonmember certificate issued", status)
		}
		return IssuedCertificate{}, key
	}
	if status != 200 {
		t.Fatal("member callback", status)
	}
	approve := url.Values{"transaction": {started.Transaction}, "csrf": {extractCSRF(t, string(page))}, "decision": {"approve"}}
	status, _, _ = request("POST", "/enroll/native/approve", "application/x-www-form-urlencoded", strings.NewReader(approve.Encode()), cookie)
	if status != 200 {
		t.Fatal("device approval", status)
	}
	wrong := proof
	wrong.Verifier, _ = randomProof()
	if status, _ := postJSON("/enroll/native/poll", wrong); status != 400 {
		t.Fatal("wrong proof consumed transaction")
	}
	status, body = postJSON("/enroll/native/poll", proof)
	var result NativePollResult
	if status != 200 || json.Unmarshal(body, &result) != nil || result.Certificate == nil {
		t.Fatal("approved native claim", status)
	}
	first := *result.Certificate
	binding, _ := ParseCSR(csr)
	if first.SPKIDigest != binding.SPKIDigest || first.Identity.DeviceID != device {
		t.Fatal("native CSR replaced")
	}
	// Simulate an unobserved first response, then recover through real HTTPS.
	time.Sleep(PollInterval)
	f.now = time.Now().UTC().Truncate(time.Second)
	status, body = postJSON("/enroll/native/poll", proof)
	result = NativePollResult{}
	if status != 200 || json.Unmarshal(body, &result) != nil || result.Certificate == nil || result.Certificate.Serial != first.Serial {
		t.Fatal("native response recovery reissued", status)
	}
	return first, key
}
