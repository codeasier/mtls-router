//go:build unix

package routerenroll

import (
	"context"
	"encoding/pem"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

type loaderFunc func(context.Context, GatewayTarget, GatewayBundle) (GatewayAck, error)

func (f loaderFunc) Load(ctx context.Context, t GatewayTarget, b GatewayBundle) (GatewayAck, error) {
	return f(ctx, t, b)
}

func TestDualGatewayPartialPublicationAndFiveMinuteDegradation(t *testing.T) {
	store, _, request, now := prepareLedger(t)
	issuer, _, root := fixtureIssuer(t, now)
	ctx := context.Background()
	issued, code := store.issueAuthorized(ctx, issuer, request, now)
	if code != "" {
		t.Fatal(code)
	}
	rootFile := filepath.Join(t.TempDir(), "root-crl.pem")
	if err := os.WriteFile(rootFile, pem.EncodeToMemory(&pem.Block{Type: "X509 CRL", Bytes: fixtureRootCRL(t, root, 1, now)}), 0600); err != nil {
		t.Fatal(err)
	}
	secondReady := false
	forged := false
	p := &Publisher{Store: store, Issuer: issuer, RootCRLFile: rootFile, Targets: []GatewayTarget{{"jump", "codeasier-crl@jump"}, {"jump2", "codeasier-crl@jump2"}}}
	p.Loader = loaderFunc(func(_ context.Context, target GatewayTarget, b GatewayBundle) (GatewayAck, error) {
		if target.ID == "jump2" && !secondReady {
			return GatewayAck{}, ErrGateway
		}
		ack := GatewayAck{target.ID, b.Publication.IssuerID, b.Publication.Number, b.Publication.Digest, Digest(b.RootCRLDER), now.Unix()}
		if forged && target.ID == "jump2" {
			ack.Digest = "unloaded-digest"
		}
		return ack, nil
	})
	status, err := p.Publish(ctx, now)
	if err != nil || status.Publication != "pending" || status.TargetExceeded || !status.Gateways[0].Confirmed || status.Gateways[1].Confirmed {
		t.Fatal("partial success reported globally", err, status)
	}
	if _, err := revoke(ctx, store, RevocationTarget{"serial", issued.Serial}, now); err != nil {
		t.Fatal(err)
	}
	now = now.Add(RevocationPublicationTarget + time.Second)
	status, err = p.Publish(ctx, now)
	if err != nil || status.Publication != "pending" || !status.TargetExceeded {
		t.Fatal("missed target not visible", err, status)
	}
	secondReady, forged = true, true
	status, err = p.Publish(ctx, now)
	if err != nil || status.Publication != "pending" {
		t.Fatal("forged digest acknowledged", err)
	}
	forged = false
	status, err = p.Publish(ctx, now)
	if err != nil || status.Publication != "loaded" || status.TargetExceeded {
		t.Fatal("actual dual ack not observed", err, status)
	}
	now = now.Add(CRLRefreshInterval)
	secondReady = false
	status, err = p.Publish(ctx, now)
	if err != nil || status.Publication != "pending" || status.TargetExceeded {
		t.Fatal("hourly refresh misreported as lost revocation", err, status)
	}
}

type fakeGatewayRuntime struct {
	marker                 string
	testErr, reloadErr     error
	tests, reloads, probes int
}

func TestRevokedIssuingCAIsPublishedWithoutSigningOrRevival(t *testing.T) {
	store, _, request, now := prepareLedger(t)
	issuer, _, root := fixtureIssuer(t, now)
	ctx := context.Background()
	file := filepath.Join(t.TempDir(), "root-crl.pem")
	write := func(number int64, revoked bool) {
		der := fixtureRootCRL(t, root, number, now)
		if revoked {
			der = fixtureRootCRL(t, root, number, now, issuer.cert)
		}
		if os.WriteFile(file, pem.EncodeToMemory(&pem.Block{Type: "X509 CRL", Bytes: der}), 0600) != nil {
			t.Fatal("root CRL fixture")
		}
	}
	write(1, false)
	p := Publisher{Store: store, Issuer: issuer, RootCRLFile: file, Targets: []GatewayTarget{{"jump", "codeasier-crl@jump"}, {"jump2", "codeasier-crl@jump2"}}, Loader: loaderFunc(func(_ context.Context, target GatewayTarget, b GatewayBundle) (GatewayAck, error) {
		return GatewayAck{target.ID, b.Publication.IssuerID, b.Publication.Number, b.Publication.Digest, Digest(b.RootCRLDER), now.Unix()}, nil
	})}
	before, err := p.Publish(ctx, now)
	if err != nil {
		t.Fatal(err)
	}
	write(2, true)
	after, err := p.Publish(ctx, now)
	if err != nil || after.Publication != "loaded" || after.Number != before.Number || issuer.Ready(now) {
		t.Fatal("root revocation was treated as invalid material instead of publishing denial", err)
	}
	if _, code := store.issueAuthorized(ctx, issuer, request, now); code != ServiceUnavailable {
		t.Fatal("revoked authority signed", code)
	}
	write(3, false)
	if _, err := p.Publish(ctx, now); err == nil {
		t.Fatal("higher root CRL removed a durable authority revocation")
	}
}

func (r *fakeGatewayRuntime) Test(context.Context) error   { r.tests++; return r.testErr }
func (r *fakeGatewayRuntime) Reload(context.Context) error { r.reloads++; return r.reloadErr }
func (r *fakeGatewayRuntime) Marker(context.Context) (string, error) {
	r.probes++
	return r.marker, nil
}

func TestGatewayValidatesAtomicallyAndRequiresObservedLoad(t *testing.T) {
	store, _, request, now := prepareLedger(t)
	issuer, _, root := fixtureIssuer(t, now)
	ctx := context.Background()
	rootDER := fixtureRootCRL(t, root, 1, now)
	pub, err := store.BuildCRL(ctx, issuer, rootDER, now)
	if err != nil {
		t.Fatal(err)
	}
	bundle := GatewayBundle{pub, rootDER}
	dir := t.TempDir()
	cfg := GatewayLoadConfig{ID: "jump", StateDir: filepath.Join(dir, "state"), IssuerCertFile: filepath.Join(dir, "issuer.pem"), RootCAFile: filepath.Join(dir, "root.pem")}
	if os.WriteFile(cfg.IssuerCertFile, certificatePEM(issuer.cert), 0600) != nil || os.WriteFile(cfg.RootCAFile, certificatePEM(root.cert), 0600) != nil {
		t.Fatal("trust files")
	}
	runtime := &fakeGatewayRuntime{marker: gatewayMarker(bundle)}
	ack, err := ApplyGatewayBundle(ctx, cfg, bundle, runtime, now)
	if err != nil || ack.Digest != pub.Digest || runtime.tests != 1 || runtime.reloads != 1 || runtime.probes != 1 {
		t.Fatal("unverified ack", err)
	}
	before, err := os.ReadFile(filepath.Join(cfg.StateDir, "current-crl.pem"))
	if err != nil {
		t.Fatal(err)
	}
	bad := bundle
	bad.Publication.DER = append([]byte(nil), pub.DER...)
	bad.Publication.DER[len(pub.DER)-1] ^= 1
	if _, err := ApplyGatewayBundle(ctx, cfg, bad, runtime, now); err == nil {
		t.Fatal("bad signature installed")
	}
	after, _ := os.ReadFile(filepath.Join(cfg.StateDir, "current-crl.pem"))
	if string(before) != string(after) {
		t.Fatal("invalid material replaced last good")
	}
	issued, code := store.issueAuthorized(ctx, issuer, request, now)
	if code != "" {
		t.Fatal(code)
	}
	if _, err := revoke(ctx, store, RevocationTarget{"serial", issued.Serial}, now); err != nil {
		t.Fatal(err)
	}
	newPub, err := store.BuildCRL(ctx, issuer, rootDER, now)
	if err != nil {
		t.Fatal(err)
	}
	next := GatewayBundle{newPub, rootDER}
	runtime.testErr = errors.New("nginx-config-canary")
	if _, err := ApplyGatewayBundle(ctx, cfg, next, runtime, now); err == nil {
		t.Fatal("failed nginx -t acknowledged")
	}
	after, _ = os.ReadFile(filepath.Join(cfg.StateDir, "current-crl.pem"))
	if string(before) != string(after) {
		t.Fatal("test failure did not preserve old files")
	}
	if _, err := ApplyGatewayBundle(ctx, cfg, bundle, runtime, now); err == nil {
		t.Fatal("high-water rolled back after failed load")
	}
	runtime.testErr = nil
	short, cancel := context.WithTimeout(ctx, 20*time.Millisecond)
	defer cancel()
	if _, err := ApplyGatewayBundle(short, cfg, next, runtime, now); err == nil {
		t.Fatal("reload signal without observed marker acknowledged")
	}
	runtime.marker = gatewayMarker(next)
	if _, err := ApplyGatewayBundle(ctx, cfg, next, runtime, now); err != nil {
		t.Fatal("same version could not resume interrupted load", err)
	}
}
