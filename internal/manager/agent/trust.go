package agent

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/codeasier/mtls-router/internal/manager/agent/modelconfig"
)

const (
	signingKeyFileName = "token-signing-key.json"
	sidecarFileName    = "last-applied-model-config.json"
	maxSigningKeySize  = 4096
)

type signingKeyFile struct {
	Version    int    `json:"version"`
	Generation string `json:"generation"`
	Key        string `json:"key"`
}

func (s *Service) ensureSigner() error {
	key, err := s.loadOrCreateSigningKey()
	return s.setSigner(key, err)
}

func (s *Service) ensureExistingSigner() error {
	key, err := loadSigningKey(filepath.Join(s.stateDir, signingKeyFileName))
	return s.setSigner(key, err)
}

func (s *Service) setSigner(key signingKeyFile, err error) error {
	if err != nil {
		return operationError(CodeModelStateInvalid, "Agent model trust state is invalid")
	}
	decoded, err := base64.RawStdEncoding.Strict().DecodeString(key.Key)
	if err != nil {
		return operationError(CodeModelStateInvalid, "Agent model trust state is invalid")
	}
	signer, err := modelconfig.NewTokenSigner(decoded, key.Generation)
	zeroBytes(decoded)
	if err != nil {
		return operationError(CodeModelStateInvalid, "Agent model trust state is invalid")
	}
	s.signer = signer
	s.keyGeneration = key.Generation
	return nil
}

func (s *Service) loadOrCreateSigningKey() (signingKeyFile, error) {
	path := filepath.Join(s.stateDir, signingKeyFileName)
	key, err := loadSigningKey(path)
	if err == nil {
		return key, nil
	}
	if !os.IsNotExist(err) {
		return signingKeyFile{}, err
	}
	if stateExists(s.journalPath()) || stateExists(filepath.Join(s.stateDir, sidecarFileName)) {
		return signingKeyFile{}, errors.New("signing key missing for existing state")
	}
	if err := s.prepareStateDir(); err != nil {
		return signingKeyFile{}, err
	}
	var material [32]byte
	if _, err := io.ReadFull(rand.Reader, material[:]); err != nil {
		return signingKeyFile{}, err
	}
	generation, err := randomID()
	if err != nil {
		zeroBytes(material[:])
		return signingKeyFile{}, err
	}
	created := signingKeyFile{Version: 1, Generation: generation, Key: base64.RawStdEncoding.EncodeToString(material[:])}
	zeroBytes(material[:])
	content, err := json.Marshal(created)
	if err != nil {
		return signingKeyFile{}, err
	}
	content = append(content, '\n')
	defer zeroBytes(content)
	tmp, err := os.CreateTemp(s.stateDir, ".token-signing-key-*")
	if err != nil {
		return signingKeyFile{}, err
	}
	tmpPath := tmp.Name()
	defer os.Remove(tmpPath)
	if err := restrictPrivate(tmpPath, false); err != nil {
		tmp.Close()
		return signingKeyFile{}, err
	}
	if _, err := tmp.Write(content); err != nil {
		tmp.Close()
		return signingKeyFile{}, err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return signingKeyFile{}, err
	}
	if err := tmp.Close(); err != nil {
		return signingKeyFile{}, err
	}
	if err := os.Link(tmpPath, path); err != nil {
		if !errors.Is(err, os.ErrExist) {
			return signingKeyFile{}, err
		}
		return loadSigningKey(path)
	}
	if err := syncDirectory(s.stateDir); err != nil {
		return signingKeyFile{}, err
	}
	return created, nil
}

func loadSigningKey(path string) (signingKeyFile, error) {
	info, err := os.Stat(path)
	if err != nil {
		return signingKeyFile{}, err
	}
	if !info.Mode().IsRegular() || !privatePermissionsOK(path, false, info.Mode()) || info.Size() <= 0 || info.Size() > maxSigningKeySize {
		return signingKeyFile{}, errors.New("unsafe signing key file")
	}
	content, err := os.ReadFile(path)
	if err != nil {
		return signingKeyFile{}, err
	}
	defer zeroBytes(content)
	var key signingKeyFile
	decoder := json.NewDecoder(strings.NewReader(string(content)))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&key); err != nil {
		return signingKeyFile{}, err
	}
	var extra any
	if err := decoder.Decode(&extra); err != io.EOF {
		return signingKeyFile{}, errors.New("trailing signing key data")
	}
	material, err := base64.RawStdEncoding.Strict().DecodeString(key.Key)
	if err != nil || len(material) != 32 || key.Version != 1 || len(key.Generation) != 32 {
		zeroBytes(material)
		return signingKeyFile{}, errors.New("invalid signing key")
	}
	zeroBytes(material)
	return key, nil
}

func stateExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil || !os.IsNotExist(err)
}
