package routerenroll

import (
	"bytes"
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"io"
	"os"
	"os/exec"
	"regexp"
	"time"
)

var ErrGateway = errors.New("enroll_gateway_unconfirmed")
var gatewayTargetPattern = regexp.MustCompile(`^[a-z_][a-z0-9_-]{0,31}@[A-Za-z0-9][A-Za-z0-9.-]{0,252}$`)

type GatewayTarget struct {
	ID     string `json:"id"`
	Target string `json:"target"`
}
type GatewayBundle struct {
	Publication CRLPublication `json:"publication"`
	RootCRLDER  []byte         `json:"root_crl_der"`
}
type GatewayAck struct {
	GatewayID  string `json:"gateway_id"`
	IssuerID   string `json:"issuer_id"`
	Number     int64  `json:"number"`
	Digest     string `json:"digest"`
	RootDigest string `json:"root_digest"`
	LoadedAt   int64  `json:"loaded_at"`
}

type GatewayLoader interface {
	Load(context.Context, GatewayTarget, GatewayBundle) (GatewayAck, error)
}

func ReadGatewayTargets(path string) ([]GatewayTarget, error) {
	data, err := readPrivateFile(path, 16*1024)
	if err != nil {
		return nil, ErrConfig
	}
	var targets []GatewayTarget
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&targets) != nil || decoder.Decode(new(any)) != io.EOF || len(targets) != 2 {
		return nil, ErrConfig
	}
	ids, destinations := map[string]bool{}, map[string]bool{}
	for _, v := range targets {
		if !stableIDPattern.MatchString(v.ID) || !gatewayTargetPattern.MatchString(v.Target) || ids[v.ID] || destinations[v.Target] {
			return nil, ErrConfig
		}
		ids[v.ID], destinations[v.Target] = true, true
	}
	return targets, nil
}

type SSHGatewayLoader struct{}
type boundedOutput struct {
	bytes.Buffer
	limit int
}

func (w *boundedOutput) Write(p []byte) (int, error) {
	if len(p) > w.limit-w.Len() {
		return 0, ErrGateway
	}
	return w.Buffer.Write(p)
}

// SSH authenticates the server against the service account's pinned known_hosts.
// The remote command is fixed; user-controlled shell fragments and fallback to
// an unverified host are deliberately absent. No private key is sent in stdin.
func (SSHGatewayLoader) Load(ctx context.Context, target GatewayTarget, bundle GatewayBundle) (GatewayAck, error) {
	if !gatewayTargetPattern.MatchString(target.Target) {
		return GatewayAck{}, ErrConfig
	}
	data, err := json.Marshal(bundle)
	if err != nil || len(data) > MaxCRLBytes*4 {
		return GatewayAck{}, ErrGateway
	}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "ssh", "-T", "-oBatchMode=yes", "-oStrictHostKeyChecking=yes", "-oConnectTimeout=5", "--", target.Target, "/usr/local/libexec/codeasier-router-enroll gateway-load")
	cmd.WaitDelay = time.Second
	cmd.Stdin = bytes.NewReader(data)
	output := &boundedOutput{limit: 4096}
	cmd.Stdout = output
	cmd.Stderr = io.Discard
	if cmd.Run() != nil {
		return GatewayAck{}, ErrGateway
	}
	var ack GatewayAck
	decoder := json.NewDecoder(bytes.NewReader(output.Bytes()))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&ack) != nil || decoder.Decode(new(any)) != io.EOF {
		return GatewayAck{}, ErrGateway
	}
	return ack, nil
}

type Publisher struct {
	Store       *Store
	Issuer      *Issuer
	RootCRLFile string
	Targets     []GatewayTarget
	Loader      GatewayLoader
}

func (p *Publisher) Status(ctx context.Context, now time.Time) (PublicationStatus, error) {
	data, err := readPublicBounded(p.RootCRLFile, MaxCRLBytes*2)
	if err != nil {
		return PublicationStatus{}, err
	}
	der, err := ParseCRLPEM(data)
	if err != nil {
		return PublicationStatus{}, err
	}
	return publicationStatus(ctx, p.Store.db, p.Issuer.id, p.Targets, Digest(der), now)
}

func readPublicBounded(path string, limit int64) ([]byte, error) {
	info, err := os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() || info.Size() > limit {
		return nil, ErrCRL
	}
	file, err := os.Open(path)
	if err != nil {
		return nil, ErrCRL
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, limit+1))
	if err != nil || int64(len(data)) > limit {
		return nil, ErrCRL
	}
	return data, nil
}

func (p *Publisher) Publish(ctx context.Context, now time.Time) (PublicationStatus, error) {
	data, err := readPublicBounded(p.RootCRLFile, MaxCRLBytes*2)
	if err != nil {
		p.Issuer.blockRootCRL()
		return PublicationStatus{}, err
	}
	rootDER, err := ParseCRLPEM(data)
	if err != nil {
		p.Issuer.blockRootCRL()
		return PublicationStatus{}, err
	}
	publication, err := p.Store.BuildCRL(ctx, p.Issuer, rootDER, now)
	if errors.Is(err, ErrIssuerRevoked) {
		p.Issuer.blockRootCRL()
		publication.IssuerID = p.Issuer.id
		err = p.Store.db.QueryRowContext(ctx, "SELECT number,digest,crl_der,this_update,next_update FROM crl_publications WHERE issuer_id=? ORDER BY number DESC LIMIT 1", p.Issuer.id).Scan(&publication.Number, &publication.Digest, &publication.DER, &publication.ThisUpdate, &publication.NextUpdate)
		if err != nil {
			return PublicationStatus{}, ErrCRL
		}
		if _, err := ValidateCRL(publication.DER, p.Issuer.cert, publication.Number, publication.Digest, now); err != nil {
			return PublicationStatus{}, err
		}
	} else if err != nil {
		p.Issuer.blockRootCRL()
		return PublicationStatus{}, err
	} else {
		if err := p.Issuer.setRootCRL(rootDER, now); err != nil {
			return PublicationStatus{}, err
		}
	}
	bundle := GatewayBundle{publication, rootDER}
	if len(p.Targets) != 2 || p.Loader == nil {
		return PublicationStatus{}, ErrConfig
	}
	for _, target := range p.Targets {
		var existingNumber int64
		var existingDigest, existingRoot string
		if p.Store.db.QueryRowContext(ctx, "SELECT number,digest,root_digest FROM gateway_acknowledgments WHERE gateway_id=? AND issuer_id=?", target.ID, p.Issuer.id).Scan(&existingNumber, &existingDigest, &existingRoot) == nil && existingNumber == publication.Number && existingDigest == publication.Digest && existingRoot == Digest(rootDER) {
			continue
		}
		ack, err := p.Loader.Load(ctx, target, bundle)
		if err != nil {
			continue
		}
		if ack.GatewayID != target.ID || ack.IssuerID != publication.IssuerID || ack.Number != publication.Number || ack.Digest != publication.Digest || ack.RootDigest != Digest(rootDER) || ack.LoadedAt < now.Add(-ClockSkew).Unix() || ack.LoadedAt > now.Add(ClockSkew).Unix() {
			continue
		}
		err = p.Store.transaction(ctx, func(tx *writeTx) error {
			_, err := tx.ExecContext(ctx, `INSERT INTO gateway_acknowledgments(gateway_id,issuer_id,number,digest,root_digest,loaded_at) VALUES(?,?,?,?,?,?) ON CONFLICT(gateway_id,issuer_id) DO UPDATE SET number=excluded.number,digest=excluded.digest,root_digest=excluded.root_digest,loaded_at=excluded.loaded_at WHERE excluded.number>=gateway_acknowledgments.number`, ack.GatewayID, ack.IssuerID, ack.Number, ack.Digest, ack.RootDigest, ack.LoadedAt)
			return err
		})
		if err != nil {
			return PublicationStatus{}, err
		}
	}
	return publicationStatus(ctx, p.Store.db, p.Issuer.id, p.Targets, Digest(rootDER), now)
}

type GatewayStatus struct {
	ID        string `json:"id"`
	Confirmed bool   `json:"confirmed"`
	LoadedAt  int64  `json:"loaded_at,omitempty"`
}
type PublicationStatus struct {
	IssuerID       string          `json:"issuer_id"`
	Number         int64           `json:"number"`
	Digest         string          `json:"digest"`
	NextUpdate     int64           `json:"next_update"`
	Publication    string          `json:"publication"`
	TargetExceeded bool            `json:"target_exceeded"`
	Gateways       []GatewayStatus `json:"gateways"`
}

func publicationStatus(ctx context.Context, db policyReader, issuer string, targets []GatewayTarget, rootDigest string, now time.Time) (PublicationStatus, error) {
	status := PublicationStatus{IssuerID: issuer, Publication: "pending", Gateways: make([]GatewayStatus, 0, len(targets))}
	if db.QueryRowContext(ctx, "SELECT number,digest,next_update FROM crl_publications WHERE issuer_id=? ORDER BY number DESC LIMIT 1", issuer).Scan(&status.Number, &status.Digest, &status.NextUpdate) != nil {
		return status, ErrStorage
	}
	root, err := rootCheckpoint(ctx, db)
	if err != nil {
		return status, err
	}
	if root.NextUpdate > 0 && root.NextUpdate < status.NextUpdate {
		status.NextUpdate = root.NextUpdate
	}
	confirmed := status.NextUpdate > now.Unix() && len(targets) == 2
	var oldest int64
	for _, target := range targets {
		var n int64
		var digest, root string
		v := GatewayStatus{ID: target.ID}
		if db.QueryRowContext(ctx, "SELECT number,digest,root_digest,loaded_at FROM gateway_acknowledgments WHERE gateway_id=? AND issuer_id=?", target.ID, issuer).Scan(&n, &digest, &root, &v.LoadedAt) == nil {
			v.Confirmed = n == status.Number && digest == status.Digest && root == rootDigest
		}
		confirmed = confirmed && v.Confirmed
		status.Gateways = append(status.Gateways, v)
		var ackDER []byte
		var ackExpiry int64
		if n > 0 {
			_ = db.QueryRowContext(ctx, "SELECT crl_der,next_update FROM crl_publications WHERE issuer_id=? AND number=? AND digest=?", issuer, n, digest).Scan(&ackDER, &ackExpiry)
		}
		if ackExpiry <= now.Unix() {
			ackDER = nil
		}
		_, missingAt, err := revocationCoverage(ctx, db, issuer, ackDER)
		if err != nil {
			return status, err
		}
		if missingAt > 0 && (oldest == 0 || missingAt < oldest) {
			oldest = missingAt
		}
	}
	// The signed CRL itself must cover every current serial, including a
	// decision racing the last build/ack. A fresh ack of an older set isn't global.
	var der []byte
	if db.QueryRowContext(ctx, "SELECT crl_der FROM crl_publications WHERE issuer_id=? AND number=?", issuer, status.Number).Scan(&der) != nil {
		return status, ErrStorage
	}
	// Coverage is evaluated with one query per absent serial below by the helper.
	covered, missingAt, err := revocationCoverage(ctx, db, issuer, der)
	if err != nil {
		return status, err
	}
	confirmed = confirmed && covered
	if !covered && missingAt > 0 && (oldest == 0 || missingAt < oldest) {
		oldest = missingAt
	}
	if confirmed {
		status.Publication = "loaded"
	} else if oldest > 0 && now.Unix()-oldest > int64(RevocationPublicationTarget.Seconds()) {
		status.TargetExceeded = true
	}
	return status, nil
}

func revocationCoverage(ctx context.Context, db policyReader, issuer string, der []byte) (bool, int64, error) {
	serials := make([]string, 0)
	if len(der) > 0 {
		crl, err := x509.ParseRevocationList(der)
		if err != nil {
			return false, 0, ErrCRL
		}
		for _, entry := range crl.RevokedCertificateEntries {
			serials = append(serials, entry.SerialNumber.Text(16))
		}
	}
	encoded, _ := json.Marshal(serials)
	var count, oldest int64
	if db.QueryRowContext(ctx, `SELECT count(*),COALESCE(min(decided_at),0) FROM revocations WHERE issuer_id=? AND serial NOT IN (SELECT value FROM json_each(?))`, issuer, string(encoded)).Scan(&count, &oldest) != nil {
		return false, 0, ErrStorage
	}
	return count == 0, oldest, nil
}

func (a *Admin) PublicationStatus(ctx context.Context, issuerFile, rootCRLFile, targetsFile string, now time.Time) (PublicationStatus, error) {
	certPEM, err := readPublicBounded(issuerFile, MaxCertificateBytes)
	if err != nil {
		return PublicationStatus{}, err
	}
	cert, err := parseSingleCertificate(certPEM)
	if err != nil {
		return PublicationStatus{}, err
	}
	rootPEM, err := readPublicBounded(rootCRLFile, MaxCRLBytes*2)
	if err != nil {
		return PublicationStatus{}, err
	}
	rootDER, err := ParseCRLPEM(rootPEM)
	if err != nil {
		return PublicationStatus{}, err
	}
	targets, err := ReadGatewayTargets(targetsFile)
	if err != nil {
		return PublicationStatus{}, err
	}
	var result PublicationStatus
	err = a.journal.locked(ctx, func() error {
		if err := a.journal.recover(ctx, a.db); err != nil {
			return err
		}
		var err error
		result, err = publicationStatus(ctx, a.db, Digest(cert.Raw), targets, Digest(rootDER), now)
		return err
	})
	return result, err
}
