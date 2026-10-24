package routerenroll

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/x509"
	"database/sql"
	"encoding/pem"
	"errors"
	"math/big"
	"time"
)

const (
	CRLRefreshInterval          = time.Hour
	CRLLifetime                 = 24 * time.Hour
	RevocationPublicationTarget = 5 * time.Minute
	MaxCRLBytes                 = 4 * 1024 * 1024
)

var ErrCRL = errors.New("enroll_crl_invalid")
var ErrIssuerRevoked = errors.New("enroll_issuing_ca_revoked")

type CRLPublication struct {
	IssuerID   string `json:"issuer_id"`
	Number     int64  `json:"number"`
	Digest     string `json:"digest"`
	DER        []byte `json:"crl_der"`
	ThisUpdate int64  `json:"this_update"`
	NextUpdate int64  `json:"next_update"`
}

// ValidateCRL verifies signed public material; a digest alone is not a trust
// decision. Equal numbers are allowed only for the identical prior CRL.
func ValidateCRL(der []byte, issuer *x509.Certificate, minimum int64, previousDigest string, now time.Time) (*x509.RevocationList, error) {
	if len(der) == 0 || len(der) > MaxCRLBytes || issuer == nil {
		return nil, ErrCRL
	}
	crl, err := x509.ParseRevocationList(der)
	// AuthorityKeyIdentifier is optional in RFC 5280. When present it must
	// bind the CRL to this exact issuer; an absent AKI is still bound by the
	// signature and RawIssuer check below.
	akiMatches := err == nil && (len(crl.AuthorityKeyId) == 0 || bytes.Equal(crl.AuthorityKeyId, issuer.SubjectKeyId))
	if err != nil || crl.CheckSignatureFrom(issuer) != nil || !bytes.Equal(crl.RawIssuer, issuer.RawSubject) || !akiMatches || crl.Number == nil || !crl.Number.IsInt64() || crl.Number.Int64() < 1 || crl.Number.Int64() < minimum || (crl.Number.Int64() == minimum && previousDigest != "" && previousDigest != Digest(der)) || crl.ThisUpdate.After(now) || !crl.NextUpdate.After(now) || !crl.NextUpdate.After(crl.ThisUpdate) || now.Before(issuer.NotBefore) || !now.Before(issuer.NotAfter) {
		return nil, ErrCRL
	}
	for _, ext := range crl.Extensions {
		if ext.Critical {
			return nil, ErrCRL
		}
	}
	seen := map[string]bool{}
	for _, entry := range crl.RevokedCertificateEntries {
		if entry.SerialNumber == nil || entry.SerialNumber.Sign() <= 0 || entry.RevocationTime.After(now.Add(ClockSkew)) || len(entry.ExtraExtensions) > 0 {
			return nil, ErrCRL
		}
		serial := entry.SerialNumber.Text(16)
		if seen[serial] {
			return nil, ErrCRL
		}
		seen[serial] = true
		for _, ext := range entry.Extensions {
			if ext.Critical {
				return nil, ErrCRL
			}
		}
	}
	return crl, nil
}

func (i *Issuer) validateRootCRL(der []byte, minimum int64, digest string, now time.Time) (*x509.RevocationList, error) {
	crl, err := ValidateCRL(der, i.root, minimum, digest, now)
	if err != nil {
		return nil, err
	}
	for _, entry := range crl.RevokedCertificateEntries {
		if entry.SerialNumber.Cmp(i.cert.SerialNumber) == 0 {
			return nil, ErrIssuerRevoked
		}
	}
	return crl, nil
}

type rootCRLCheckpoint struct {
	Number         int64    `json:"number"`
	Digest         string   `json:"digest"`
	RevokedSerials []string `json:"revoked_serials,omitempty"`
	NextUpdate     int64    `json:"next_update,omitempty"`
}

// BuildCRL pins the offline-root CRL monotonically and signs only from durable
// revocation decisions. A new decision forces publication without waiting for
// the hourly refresh; generation and number allocation share the journal gate.
func (s *Store) BuildCRL(ctx context.Context, issuer *Issuer, rootDER []byte, now time.Time) (CRLPublication, error) {
	revoked, err := s.acceptRootCRL(ctx, issuer, rootDER, now)
	if err != nil {
		return CRLPublication{}, err
	}
	if revoked {
		return CRLPublication{}, ErrIssuerRevoked
	}
	root, err := issuer.validateRootCRL(rootDER, 0, "", now)
	if err != nil {
		return CRLPublication{}, err
	}
	var publication CRLPublication
	err = s.transaction(ctx, func(tx *writeTx) error {
		if err := issuerAllowed(ctx, tx, issuer.cert); err != nil {
			return err
		}
		if !issuer.validChain(now) {
			return ErrCRL
		}
		rows, err := tx.QueryContext(ctx, "SELECT serial,decided_at FROM revocations WHERE issuer_id=? ORDER BY serial", issuer.id)
		if err != nil {
			return ErrStorage
		}
		var entries []x509.RevocationListEntry
		for rows.Next() {
			var serial string
			var at int64
			if rows.Scan(&serial, &at) != nil {
				_ = rows.Close()
				return ErrStorage
			}
			n, ok := new(big.Int).SetString(serial, 16)
			if !ok || n.Sign() <= 0 || at > now.Add(ClockSkew).Unix() {
				_ = rows.Close()
				return ErrCRL
			}
			entries = append(entries, x509.RevocationListEntry{SerialNumber: n, RevocationTime: time.Unix(at, 0), ReasonCode: 0})
		}
		err = rows.Err()
		_ = rows.Close()
		if err != nil {
			return ErrStorage
		}
		var latest CRLPublication
		err = tx.QueryRowContext(ctx, "SELECT number,digest,crl_der,this_update,next_update FROM crl_publications WHERE issuer_id=? ORDER BY number DESC LIMIT 1", issuer.id).Scan(&latest.Number, &latest.Digest, &latest.DER, &latest.ThisUpdate, &latest.NextUpdate)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return ErrStorage
		}
		if err == nil {
			latest.IssuerID = issuer.id
			parsed, err := x509.ParseRevocationList(latest.DER)
			if err != nil || parsed.CheckSignatureFrom(issuer.cert) != nil || parsed.Number == nil || parsed.Number.Int64() != latest.Number || Digest(latest.DER) != latest.Digest {
				return ErrCRL
			}
			previous, err := ValidateCRL(latest.DER, issuer.cert, latest.Number, latest.Digest, now)
			if err == nil && now.Unix() < latest.ThisUpdate+int64(CRLRefreshInterval.Seconds()) && sameRevokedSerials(previous.RevokedCertificateEntries, entries) {
				publication = latest
				return nil
			}
		}
		if latest.Number == int64(^uint64(0)>>1) {
			return ErrCRL
		}
		next := now.Add(CRLLifetime)
		for _, limit := range []time.Time{issuer.cert.NotAfter, issuer.root.NotAfter, root.NextUpdate} {
			if next.After(limit) {
				next = limit
			}
		}
		if !next.After(now) {
			return ErrCRL
		}
		template := &x509.RevocationList{Number: big.NewInt(latest.Number + 1), ThisUpdate: now.UTC().Truncate(time.Second), NextUpdate: next.UTC().Truncate(time.Second), RevokedCertificateEntries: entries}
		der, err := x509.CreateRevocationList(rand.Reader, template, issuer.cert, issuer.key)
		if err != nil || len(der) > MaxCRLBytes {
			return ErrCRL
		}
		publication = CRLPublication{issuer.id, template.Number.Int64(), Digest(der), der, template.ThisUpdate.Unix(), template.NextUpdate.Unix()}
		_, err = tx.ExecContext(ctx, "INSERT INTO crl_publications(issuer_id,number,digest,crl_der,this_update,next_update,created_at) VALUES(?,?,?,?,?,?,?)", issuer.id, publication.Number, publication.Digest, der, publication.ThisUpdate, publication.NextUpdate, now.Unix())
		return err
	})
	if err != nil {
		return CRLPublication{}, err
	}
	return publication, nil
}

func sameRevokedSerials(a, b []x509.RevocationListEntry) bool {
	if len(a) != len(b) {
		return false
	}
	set := make(map[string]bool, len(a))
	for _, v := range a {
		set[v.SerialNumber.Text(16)] = true
	}
	for _, v := range b {
		if !set[v.SerialNumber.Text(16)] {
			return false
		}
	}
	return true
}

func ParseCRLPEM(value []byte) ([]byte, error) {
	if len(value) > MaxCRLBytes*2 {
		return nil, ErrCRL
	}
	block, rest := pem.Decode(value)
	if block == nil || block.Type != "X509 CRL" || len(block.Headers) != 0 || len(bytes.TrimSpace(rest)) != 0 || len(block.Bytes) > MaxCRLBytes {
		return nil, ErrCRL
	}
	return block.Bytes, nil
}
