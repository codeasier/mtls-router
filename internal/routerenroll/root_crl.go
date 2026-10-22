package routerenroll

import (
	"context"
	"crypto/x509"
	"database/sql"
	"encoding/json"
	"encoding/pem"
	"errors"
	"sort"
	"time"
)

func revokedSerials(crl *x509.RevocationList) []string {
	result := make([]string, 0, len(crl.RevokedCertificateEntries))
	for _, v := range crl.RevokedCertificateEntries {
		result = append(result, v.SerialNumber.Text(16))
	}
	sort.Strings(result)
	return result
}
func revocationsRetained(previous, current []string) bool {
	set := make(map[string]bool, len(current))
	for _, v := range current {
		set[v] = true
	}
	for _, v := range previous {
		if !set[v] {
			return false
		}
	}
	return true
}
func rootCheckpoint(ctx context.Context, reader policyReader) (rootCRLCheckpoint, error) {
	var value rootCRLCheckpoint
	var encoded []byte
	err := reader.QueryRowContext(ctx, "SELECT value FROM metadata WHERE name='root_crl_checkpoint'").Scan(&encoded)
	if errors.Is(err, sql.ErrNoRows) {
		return value, nil
	}
	if err != nil || json.Unmarshal(encoded, &value) != nil {
		return value, ErrStorage
	}
	return value, nil
}

// A valid root revocation is durable even though it disables the issuing key.
// Dedicated root CRLs are additive: a later number cannot revive an issuer.
func (s *Store) acceptRootCRL(ctx context.Context, issuer *Issuer, der []byte, now time.Time) (bool, error) {
	revoked := false
	err := s.transaction(ctx, func(tx *writeTx) error {
		prior, err := rootCheckpoint(ctx, tx)
		if err != nil {
			return err
		}
		root, err := ValidateCRL(der, issuer.root, prior.Number, prior.Digest, now)
		if err != nil {
			return err
		}
		serials := revokedSerials(root)
		if !revocationsRetained(prior.RevokedSerials, serials) {
			return ErrCRL
		}
		for _, serial := range serials {
			if serial == issuer.cert.SerialNumber.Text(16) {
				revoked = true
			}
		}
		next := rootCRLCheckpoint{Number: root.Number.Int64(), Digest: Digest(der), RevokedSerials: serials, NextUpdate: root.NextUpdate.Unix()}
		if next.Number == prior.Number && next.Digest == prior.Digest && next.NextUpdate == prior.NextUpdate && len(next.RevokedSerials) == len(prior.RevokedSerials) {
			return nil
		}
		encoded, _ := json.Marshal(next)
		_, err = tx.ExecContext(ctx, "INSERT INTO metadata(name,value) VALUES('root_crl_checkpoint',?) ON CONFLICT(name) DO UPDATE SET value=excluded.value", encoded)
		return err
	})
	return revoked, err
}
func issuerAllowed(ctx context.Context, reader policyReader, issuer *x509.Certificate) error {
	root, err := rootCheckpoint(ctx, reader)
	if err != nil {
		return err
	}
	for _, serial := range root.RevokedSerials {
		if serial == issuer.SerialNumber.Text(16) {
			return ErrIssuerRevoked
		}
	}
	return nil
}
func certificateIssuerAllowed(ctx context.Context, reader policyReader, value IssuedCertificate) error {
	block, _ := pem.Decode([]byte(value.ChainPEM))
	if block == nil || block.Type != "CERTIFICATE" {
		return ErrCRL
	}
	issuer, err := x509.ParseCertificate(block.Bytes)
	if err != nil || Digest(issuer.Raw) != value.IssuerID {
		return ErrCRL
	}
	return issuerAllowed(ctx, reader, issuer)
}
