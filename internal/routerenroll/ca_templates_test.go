//go:build unix

package routerenroll

import (
	"context"
	"crypto/elliptic"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

func TestOfflineRootTemplateProducesAcceptedRestrictedIssuer(t *testing.T) {
	tool, err := exec.LookPath("openssl")
	if err != nil {
		t.Skip("OpenSSL unavailable; template validation requires a native OpenSSL host")
	}
	dir := t.TempDir()
	if err := os.Chmod(dir, 0700); err != nil {
		t.Fatal(err)
	}
	config, err := os.ReadFile("../../deploy/enroll/offline-root.cnf")
	if err != nil {
		t.Fatal(err)
	}
	for name, value := range map[string][]byte{"offline-root.cnf": config, "index.txt": {}, "serial": []byte("1000\n"), "crlnumber": []byte("1000\n")} {
		if err := os.WriteFile(filepath.Join(dir, name), value, 0600); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Mkdir(filepath.Join(dir, "newcerts"), 0700); err != nil {
		t.Fatal(err)
	}
	run := func(args ...string) {
		t.Helper()
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, tool, args...)
		cmd.Dir = dir
		if err := cmd.Run(); err != nil {
			t.Fatalf("OpenSSL template step %s failed: %v", args[0], err)
		}
	}
	// Unencrypted synthetic test root avoids a TTY. Operational instructions use
	// an encrypted offline root and never deploy it to the signing service.
	run("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", "root-key.pem")
	run("req", "-new", "-x509", "-sha256", "-days", "30", "-config", "offline-root.cnf", "-extensions", "root_ca_ext", "-key", "root-key.pem", "-out", "root-cert.pem")
	run("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", "issuing-key.pem")
	run("req", "-new", "-sha256", "-key", "issuing-key.pem", "-subj", "/CN=CodeasierRouter-Personal-Issuer", "-out", "issuing.csr")
	run("ca", "-batch", "-config", "offline-root.cnf", "-extensions", "issuing_ca_ext", "-days", "15", "-in", "issuing.csr", "-out", "issuing-cert.pem", "-notext")
	run("ca", "-config", "offline-root.cnf", "-gencrl", "-out", "root-crl.pem")
	if err := os.Chmod(filepath.Join(dir, "issuing-key.pem"), 0600); err != nil {
		t.Fatal(err)
	}
	issuer, err := LoadIssuer(filepath.Join(dir, "issuing-cert.pem"), filepath.Join(dir, "issuing-key.pem"), filepath.Join(dir, "root-cert.pem"), time.Now())
	if err != nil {
		t.Fatal("template did not satisfy restricted issuer profile", err)
	}
	if _, err := issuer.Issue(testCSR(t, elliptic.P256()), fixtureIdentity(), time.Now()); err != nil {
		t.Fatal("template issuer cannot issue fixture", err)
	}
}
