package routerenroll

import (
	"bytes"
	"context"
	"crypto/x509"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

type GatewayLoadConfig struct{ ID, StateDir, IssuerCertFile, RootCAFile, NginxBinary string }
type gatewayHighWater struct {
	IssuerID           string   `json:"issuer_id"`
	Number             int64    `json:"number"`
	Digest             string   `json:"digest"`
	RootNumber         int64    `json:"root_number"`
	RootDigest         string   `json:"root_digest"`
	RootRevokedSerials []string `json:"root_revoked_serials,omitempty"`
}

type GatewayRuntime interface {
	Test(context.Context) error
	Reload(context.Context) error
	Marker(context.Context) (string, error)
}

type NginxRuntime struct {
	Binary, PidFile          string
	prefix, config, probeURL string
}

func (n NginxRuntime) command(ctx context.Context, args ...string) error {
	if !filepath.IsAbs(n.Binary) {
		return ErrGateway
	}
	if n.prefix != "" {
		args = append([]string{"-p", n.prefix, "-c", n.config}, args...)
	}
	cmd := exec.CommandContext(ctx, n.Binary, args...)
	cmd.Stdout = io.Discard
	cmd.Stderr = io.Discard
	cmd.WaitDelay = time.Second
	if cmd.Run() != nil {
		return ErrGateway
	}
	return nil
}
func (n NginxRuntime) Test(ctx context.Context) error { return n.command(ctx, "-t") }
func (n NginxRuntime) Reload(ctx context.Context) error {
	workers, err := n.reloadWorkers(ctx)
	if err != nil {
		return ErrGateway
	}
	listening, err := nginxListeningWorkers(ctx, workers)
	if err != nil || len(listening) == 0 {
		return ErrGateway
	}
	if err := n.command(ctx, "-s", "reload"); err != nil {
		return err
	}
	// nginx starts new workers before asking old workers to quit. Observing
	// one new marker alone does not prove that old workers stopped accepting.
	// Wait only for their listening sockets, never their in-flight SSE.
	return waitNginxWorkersRetired(ctx, workers, nginxListeningWorkers)
}
func (n NginxRuntime) Marker(ctx context.Context) (string, error) {
	transport := &http.Transport{DisableKeepAlives: true}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	probe := "http://127.0.0.1:19102/crl-loaded"
	if n.probeURL != "" {
		probe = n.probeURL
	}
	request, _ := http.NewRequestWithContext(ctx, "GET", probe, nil)
	response, err := client.Do(request)
	if err != nil {
		return "", ErrGateway
	}
	defer response.Body.Close()
	data, err := io.ReadAll(io.LimitReader(response.Body, 1025))
	if err != nil || len(data) > 1024 || response.StatusCode != 200 {
		return "", ErrGateway
	}
	return string(data), nil
}

func DecodeGatewayBundle(input io.Reader) (GatewayBundle, error) {
	data, err := io.ReadAll(io.LimitReader(input, MaxCRLBytes*4+1))
	if err != nil || len(data) > MaxCRLBytes*4 {
		return GatewayBundle{}, ErrCRL
	}
	var bundle GatewayBundle
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&bundle) != nil || decoder.Decode(new(any)) != io.EOF {
		return GatewayBundle{}, ErrCRL
	}
	return bundle, nil
}

func gatewayMarker(b GatewayBundle) string {
	return fmt.Sprintf("%s:%d:%s:%s\n", b.Publication.IssuerID, b.Publication.Number, b.Publication.Digest, Digest(b.RootCRLDER))
}

// ApplyGatewayBundle returns an ack only after a new TCP request observes the
// marker compiled into the same nginx config generation as ssl_crl. A signal's
// exit status or an on-disk file alone never counts as a load acknowledgment.
func ApplyGatewayBundle(ctx context.Context, cfg GatewayLoadConfig, bundle GatewayBundle, runtime GatewayRuntime, now time.Time) (GatewayAck, error) {
	if !stableIDPattern.MatchString(cfg.ID) || !filepath.IsAbs(cfg.StateDir) || runtime == nil || privateDirectory(cfg.StateDir) != nil {
		return GatewayAck{}, ErrConfig
	}
	lock, err := acquireWriterLock(filepath.Join(cfg.StateDir, "loader.lock"))
	if err != nil {
		return GatewayAck{}, err
	}
	defer lock.Close()
	issuerPEM, err := readPublicBounded(cfg.IssuerCertFile, MaxCertificateBytes)
	if err != nil {
		return GatewayAck{}, ErrCRL
	}
	rootPEM, err := readPublicBounded(cfg.RootCAFile, MaxCertificateBytes)
	if err != nil {
		return GatewayAck{}, ErrCRL
	}
	issuer, err := parseSingleCertificate(issuerPEM)
	if err != nil {
		return GatewayAck{}, ErrCRL
	}
	root, err := parseSingleCertificate(rootPEM)
	if err != nil {
		return GatewayAck{}, ErrCRL
	}
	if issuer.CheckSignatureFrom(root) != nil || !issuer.IsCA || !issuer.MaxPathLenZero || issuer.MaxPathLen != 0 || len(issuer.ExtKeyUsage) != 1 || issuer.ExtKeyUsage[0] != x509.ExtKeyUsageClientAuth {
		return GatewayAck{}, ErrCRL
	}
	authority := &Issuer{cert: issuer, root: root}
	if !authority.Ready(now) {
		return GatewayAck{}, ErrCRL
	}
	var high gatewayHighWater
	highFile := filepath.Join(cfg.StateDir, "high-water.json")
	encoded, err := readPrivateFile(highFile, 4096)
	if err == nil {
		if json.Unmarshal(encoded, &high) != nil || high.IssuerID != Digest(issuer.Raw) || high.Number < 1 || high.RootNumber < 1 {
			return GatewayAck{}, ErrCRL
		}
	} else if _, statErr := os.Lstat(highFile); !os.IsNotExist(statErr) {
		return GatewayAck{}, ErrCRL
	} else if _, activeErr := os.Lstat(filepath.Join(cfg.StateDir, "current-crl.pem")); !os.IsNotExist(activeErr) {
		return GatewayAck{}, ErrCRL
	}
	pub := bundle.Publication
	crl, err := ValidateCRL(pub.DER, issuer, high.Number, high.Digest, now)
	if err != nil || pub.IssuerID != Digest(issuer.Raw) || pub.Digest != Digest(pub.DER) || pub.Number != crl.Number.Int64() || pub.ThisUpdate != crl.ThisUpdate.Unix() || pub.NextUpdate != crl.NextUpdate.Unix() {
		return GatewayAck{}, ErrCRL
	}
	rootCRL, err := ValidateCRL(bundle.RootCRLDER, authority.root, high.RootNumber, high.RootDigest, now)
	if err != nil {
		return GatewayAck{}, ErrCRL
	}
	serials := revokedSerials(rootCRL)
	if !revocationsRetained(high.RootRevokedSerials, serials) || crl.NextUpdate.Sub(crl.ThisUpdate) > CRLLifetime {
		return GatewayAck{}, ErrCRL
	}
	// The high-water mark is fsynced before installing public material. A crash
	// or a failed reload can be retried with this same bundle, never an older one.
	high = gatewayHighWater{pub.IssuerID, pub.Number, pub.Digest, rootCRL.Number.Int64(), Digest(bundle.RootCRLDER), serials}
	encoded, _ = json.Marshal(high)
	if atomicGatewayFile(highFile, encoded) != nil {
		return GatewayAck{}, ErrStorage
	}
	activeFile, markerFile := filepath.Join(cfg.StateDir, "current-crl.pem"), filepath.Join(cfg.StateDir, "loaded-marker.conf")
	oldCRL, _ := readPrivateFile(activeFile, MaxCRLBytes*4)
	oldMarker, _ := readPrivateFile(markerFile, 4096)
	combined := append(pem.EncodeToMemory(&pem.Block{Type: "X509 CRL", Bytes: pub.DER}), pem.EncodeToMemory(&pem.Block{Type: "X509 CRL", Bytes: bundle.RootCRLDER})...)
	marker := gatewayMarker(bundle)
	include := []byte("location = /crl-loaded { default_type text/plain; return 200 \"" + pub.IssuerID + ":" + fmt.Sprint(pub.Number) + ":" + pub.Digest + ":" + Digest(bundle.RootCRLDER) + "\\n\"; }\n")
	if atomicGatewayFile(activeFile, combined) != nil || atomicGatewayFile(markerFile, include) != nil {
		return GatewayAck{}, ErrStorage
	}
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	if runtime.Test(ctx) != nil {
		// nginx has not been signalled. Restore the last tested files, but retain
		// the high-water mark so an older signed CRL cannot be submitted later.
		if len(oldCRL) > 0 {
			_ = atomicGatewayFile(activeFile, oldCRL)
		} else {
			_ = os.Remove(activeFile)
		}
		if len(oldMarker) > 0 {
			_ = atomicGatewayFile(markerFile, oldMarker)
		} else {
			_ = os.Remove(markerFile)
		}
		_ = syncDirectory(cfg.StateDir)
		return GatewayAck{}, ErrGateway
	}
	if runtime.Reload(ctx) != nil {
		return GatewayAck{}, ErrGateway
	}
	tick := time.NewTicker(50 * time.Millisecond)
	defer tick.Stop()
	for {
		observed, err := runtime.Marker(ctx)
		if err == nil && observed == marker {
			// Check that the observed tag still corresponds to installed bytes.
			actual, err := readPrivateFile(activeFile, MaxCRLBytes*4)
			if err != nil || !bytes.Equal(actual, combined) {
				return GatewayAck{}, ErrGateway
			}
			return GatewayAck{cfg.ID, pub.IssuerID, pub.Number, pub.Digest, Digest(bundle.RootCRLDER), now.Unix()}, nil
		}
		select {
		case <-ctx.Done():
			return GatewayAck{}, ErrGateway
		case <-tick.C:
		}
	}
}

func atomicGatewayFile(path string, data []byte) error {
	file, err := os.CreateTemp(filepath.Dir(path), ".crl-stage-")
	if err != nil {
		return ErrStorage
	}
	defer file.Close()
	tmp := file.Name()
	defer os.Remove(tmp)
	if file.Chmod(0600) != nil {
		_ = file.Close()
		return ErrStorage
	}
	if _, err = file.Write(data); err != nil {
		_ = file.Close()
		return ErrStorage
	}
	if file.Sync() != nil || file.Close() != nil {
		return ErrStorage
	}
	if os.Rename(tmp, path) != nil || syncDirectory(filepath.Dir(path)) != nil {
		return ErrStorage
	}
	return nil
}
