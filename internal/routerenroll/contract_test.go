package routerenroll

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"net/http/httptest"
	"net/url"
	"os"
	"reflect"
	"strings"
	"testing"
)

func protocolFixture(t *testing.T) map[string]json.RawMessage {
	t.Helper()
	data, err := os.ReadFile("../../testdata/router-mtls-v1/protocol.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture map[string]json.RawMessage
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	return fixture
}

func testCSR(t *testing.T, curve elliptic.Curve) string {
	t.Helper()
	key, err := ecdsa.GenerateKey(curve, rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{Subject: pkix.Name{CommonName: "untrusted-admin"}, DNSNames: []string{"attacker.invalid"}}, key)
	if err != nil {
		t.Fatal(err)
	}
	return string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: der}))
}

func TestSharedNativeContract(t *testing.T) {
	f := protocolFixture(t)
	var origins map[string]string
	if err := json.Unmarshal(f["origins"], &origins); err != nil {
		t.Fatal(err)
	}
	if origins["enroll"] != EnrollOrigin || origins["relay"] != RelayOrigin || origins["enroll"] == origins["relay"] {
		t.Fatal("origin contract drift")
	}
	var native struct {
		TTL              int64    `json:"ttl_seconds"`
		Poll             int64    `json:"poll_seconds"`
		Recovery         int64    `json:"recovery_seconds"`
		Fields           []string `json:"start_fields"`
		Forbidden        []string `json:"start_forbidden_fields"`
		BrowserForbidden []string `json:"browser_forbidden_fields"`
		Vector           struct {
			Input string `json:"input_utf8"`
			SHA   string `json:"sha256"`
			S256  string `json:"s256"`
		} `json:"digest_vector"`
	}
	if err := json.Unmarshal(f["native"], &native); err != nil {
		t.Fatal(err)
	}
	if native.TTL != int64(EnrollmentTTL.Seconds()) || native.Poll != int64(PollInterval.Seconds()) || native.Recovery != int64(RecoveryWindow.Seconds()) {
		t.Fatal("time contract drift")
	}
	if Digest([]byte(native.Vector.Input)) != native.Vector.SHA || Challenge(native.Vector.Input) != native.Vector.S256 {
		t.Fatal("digest encoding drift")
	}
	var fields []string
	typ := reflect.TypeFor[NativeStart]()
	for i := 0; i < typ.NumField(); i++ {
		fields = append(fields, typ.Field(i).Tag.Get("json"))
	}
	if !reflect.DeepEqual(fields, native.Fields) {
		t.Fatalf("start fields drift: %v", fields)
	}
	for _, field := range native.Forbidden {
		decoder := json.NewDecoder(strings.NewReader(`{"` + field + `":"secret-canary"}`))
		decoder.DisallowUnknownFields()
		if decoder.Decode(&NativeStart{}) == nil {
			t.Fatalf("accepted forbidden field %s", field)
		}
	}
	transaction := base64.RawURLEncoding.EncodeToString(bytes.Repeat([]byte{17}, 32))
	link, code := VerificationURL(transaction)
	parsed, err := url.Parse(link)
	if code != "" || err != nil || parsed.Scheme+"://"+parsed.Host != EnrollOrigin || len(parsed.Query()) != 1 || parsed.Query().Get("transaction") != transaction {
		t.Fatal("unsafe browser link")
	}
	for _, field := range native.BrowserForbidden {
		if parsed.Query().Has(field) {
			t.Fatal("browser secret", field)
		}
	}
	if _, code = VerificationURL("https://attacker.invalid/?verifier=canary"); code != InvalidRequest {
		t.Fatal("accepted injected link")
	}
}

func TestCSRAndProofBinding(t *testing.T) {
	csr := testCSR(t, elliptic.P256())
	binding, code := ParseCSR(csr)
	if code != "" || binding.CSRDigest == binding.SPKIDigest || len(binding.SPKIDigest) != 64 {
		t.Fatal("invalid binding")
	}
	variant := strings.ReplaceAll(csr, "\n", "\r\n")
	// PEM line-ending normalization must not change the DER digest.
	other, code := ParseCSR(variant)
	if code != "" || other.CSRDigest != binding.CSRDigest || other.SPKIDigest != binding.SPKIDigest {
		t.Fatal("PEM representation changed binding")
	}
	for _, bad := range []string{csr + csr, "prefix\n" + csr, testCSR(t, elliptic.P384()), strings.Repeat("x", MaxCSRBytes+1)} {
		if _, code := ParseCSR(bad); code != InvalidRequest {
			t.Fatal("accepted unsupported CSR")
		}
	}
	verifier := base64.RawURLEncoding.EncodeToString(bytes.Repeat([]byte{42}, 32))
	if !VerifyProof(verifier, Challenge(verifier)) || VerifyProof(verifier, Challenge(verifier+"x")) || VerifyProof("abc", Challenge("abc")) {
		t.Fatal("proof not bound")
	}
	var identity Identity
	if err := json.Unmarshal(protocolFixture(t)["identity"], &identity); err != nil {
		t.Fatal(err)
	}
	start := NativeStart{ProtocolVersion, csr, identity.DeviceID, "linux_cli", "fixture-device", Challenge(verifier), "S256"}
	if _, code := start.Validate(); code != "" {
		t.Fatal(code)
	}
	start.DeviceLabel = "forged\nlog"
	if _, code := start.Validate(); code != InvalidRequest {
		t.Fatal("accepted unsafe label")
	}
}

func TestPublicErrorsAreClosed(t *testing.T) {
	var cases []struct {
		Code   string `json:"code"`
		Status int    `json:"status"`
		Error  string `json:"error"`
	}
	if err := json.Unmarshal(protocolFixture(t)["errors"], &cases); err != nil {
		t.Fatal(err)
	}
	if len(cases) != len(publicErrors) {
		t.Fatal("error contract drift")
	}
	for _, test := range cases {
		w := httptest.NewRecorder()
		WriteError(w, test.Code)
		var body map[string]string
		if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
			t.Fatal(err)
		}
		if w.Code != test.Status || body["code"] != test.Code || body["error"] != test.Error || w.Header().Get("Cache-Control") != "no-store" {
			t.Fatalf("error drift: %s", test.Code)
		}
	}
	w := httptest.NewRecorder()
	WriteError(w, "oauth-code-private-key-api-key-canary")
	if w.Code != 503 || strings.Contains(w.Body.String(), "canary") {
		t.Fatal("raw diagnostic leaked")
	}
}
