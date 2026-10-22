package routerenroll

import (
	"context"
	"database/sql"
	"errors"
	"regexp"
	"time"
)

var ErrConfig = errors.New("enroll_config_invalid")
var orgPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`)
var stableIDPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`)

type Config struct {
	Listen           string
	StateDir         string
	TokenKeyFile     string
	ClientID         string
	ClientSecretFile string
	OrgPath          string
	ExpectedOrgID    string
	IssuerCertFile   string
	IssuerKeyFile    string
	RootCAFile       string
	JournalDir       string
	GatewaySocket    string
	RootCRLFile      string
	GatewaysFile     string
}

func LoadConfig(env func(string) (string, bool)) Config {
	get := func(key, fallback string) string {
		if value, exists := env(key); exists {
			return value
		}
		return fallback
	}
	return Config{
		Listen: get("MTLS_ENROLL_LISTEN", DefaultListen), StateDir: get("MTLS_ENROLL_STATE_DIR", ""), TokenKeyFile: get("MTLS_ENROLL_TOKEN_KEY_FILE", ""),
		ClientID: get("MTLS_ENROLL_CLIENT_ID", ""), ClientSecretFile: get("MTLS_ENROLL_CLIENT_SECRET_FILE", ""),
		OrgPath: get("MTLS_ENROLL_ORG_PATH", get("GITCODE_ORG_PATH", "msinsight-org")), ExpectedOrgID: get("MTLS_ENROLL_ORG_ID", ""),
		IssuerCertFile: get("MTLS_ENROLL_ISSUER_CERT_FILE", ""), IssuerKeyFile: get("MTLS_ENROLL_ISSUER_KEY_FILE", ""), RootCAFile: get("MTLS_ENROLL_ROOT_CA_FILE", ""),
		JournalDir:    get("MTLS_ENROLL_JOURNAL_DIR", ""),
		GatewaySocket: get("MTLS_ENROLL_GATEWAY_SOCKET", ""),
		RootCRLFile:   get("MTLS_ENROLL_ROOT_CRL_FILE", ""),
		GatewaysFile:  get("MTLS_ENROLL_GATEWAYS_FILE", ""),
	}
}

func (c Config) Validate() error {
	if ValidateListen(c.Listen) != nil || c.StateDir == "" || c.JournalDir == "" || c.TokenKeyFile == "" || !stableIDPattern.MatchString(c.ClientID) || c.ClientSecretFile == "" || !orgPattern.MatchString(c.OrgPath) || (c.ExpectedOrgID != "" && !stableIDPattern.MatchString(c.ExpectedOrgID)) {
		return ErrConfig
	}
	return nil
}

type Policy struct {
	Path, ExpectedID, BoundID, Fingerprint string
	Epoch                                  int64
}

func (p Policy) organizationID() string {
	if p.BoundID != "" {
		return p.BoundID
	}
	return p.ExpectedID
}

type policyReader interface {
	QueryRowContext(context.Context, string, ...any) *sql.Row
}

func readPolicy(ctx context.Context, reader policyReader) (Policy, error) {
	var p Policy
	err := reader.QueryRowContext(ctx, "SELECT path,expected_id,bound_id,fingerprint,epoch FROM organization_policy WHERE singleton=1").Scan(&p.Path, &p.ExpectedID, &p.BoundID, &p.Fingerprint, &p.Epoch)
	return p, err
}

// ActivatePolicy invalidates pending operations and records revocation intent
// atomically. This is not evidence that any gateway has loaded a new CRL.
func (s *Store) ActivatePolicy(ctx context.Context, cfg Config) (Policy, error) {
	if cfg.Validate() != nil {
		return Policy{}, ErrConfig
	}
	fingerprint := Digest([]byte(cfg.OrgPath + "\x00" + cfg.ExpectedOrgID + "\x00" + EnrollOrigin + "\x00" + RelayOrigin))
	var policy Policy
	err := s.transaction(ctx, func(tx *writeTx) error {
		previous, err := readPolicy(ctx, tx)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return ErrStorage
		}
		if err == nil && previous.Fingerprint == fingerprint {
			policy = previous
			return nil
		}
		policy = Policy{Path: cfg.OrgPath, ExpectedID: cfg.ExpectedOrgID, Fingerprint: fingerprint, Epoch: previous.Epoch + 1}
		if _, err := tx.ExecContext(ctx, `INSERT INTO organization_policy(singleton,path,expected_id,bound_id,fingerprint,epoch) VALUES(1,?,?,'',?,?) ON CONFLICT(singleton) DO UPDATE SET path=excluded.path,expected_id=excluded.expected_id,bound_id='',fingerprint=excluded.fingerprint,epoch=excluded.epoch`, policy.Path, policy.ExpectedID, fingerprint, policy.Epoch); err != nil {
			return ErrStorage
		}
		if _, err := tx.ExecContext(ctx, `INSERT OR IGNORE INTO revocations(issuer_id,serial,decided_at,reason) SELECT issuer_id,serial,?,'policy_changed' FROM certificates WHERE epoch<>?`, time.Now().Unix(), policy.Epoch); err != nil {
			return ErrStorage
		}
		if _, err := tx.ExecContext(ctx, "UPDATE devices SET disabled=1,version=version+1 WHERE epoch<>?", policy.Epoch); err != nil {
			return ErrStorage
		}
		if _, err := tx.ExecContext(ctx, "UPDATE enrollment_transactions SET status='expired',version=version+1 WHERE epoch<>? AND status<>'issued'", policy.Epoch); err != nil {
			return ErrStorage
		}
		return nil
	})
	return policy, err
}

func bindOrganization(ctx context.Context, tx *writeTx, epoch int64, org Organization) error {
	p, err := readPolicy(ctx, tx)
	if err != nil || p.Epoch != epoch || org.Path != p.Path || !stableIDPattern.MatchString(org.ID) || (p.organizationID() != "" && p.organizationID() != org.ID) {
		return ErrStorage
	}
	if p.BoundID == "" {
		if _, err := tx.ExecContext(ctx, "UPDATE organization_policy SET bound_id=? WHERE singleton=1 AND epoch=?", org.ID, epoch); err != nil {
			return ErrStorage
		}
	}
	return nil
}
