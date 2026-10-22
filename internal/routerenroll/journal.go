package routerenroll

import (
	"bufio"
	"bytes"
	"context"
	"crypto/rand"
	"database/sql"
	"database/sql/driver"
	"encoding/base64"
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
)

var ErrContinuity = errors.New("enroll_recovery_continuity_unavailable")

const maxJournalRecord = 8 * 1024 * 1024
const journalName = "commits.jsonl"

type journalHeader struct {
	Version int    `json:"version"`
	StoreID string `json:"store_id"`
}
type journalValue struct {
	Kind    string `json:"kind"`
	Text    string `json:"text,omitempty"`
	Integer int64  `json:"integer,omitempty"`
	Blob    []byte `json:"blob,omitempty"`
}
type journalStatement struct {
	SQL  string         `json:"sql"`
	Args []journalValue `json:"args"`
}
type journalRecord struct {
	StoreID    string             `json:"store_id"`
	Sequence   int64              `json:"sequence"`
	Previous   string             `json:"previous"`
	Statements []journalStatement `json:"statements"`
	Hash       string             `json:"hash"`
}
type RecoveryCheckpoint struct {
	StoreID  string `json:"store_id"`
	Sequence int64  `json:"sequence"`
	Hash     string `json:"hash"`
}

type journal struct {
	file, commitLock *os.File
	gate             chan struct{}
	header           journalHeader
	head             RecoveryCheckpoint
	offset           int64
	// Tests inject a crash after the authority commit, before SQLite commit.
	afterAppend func() error
}

// InitJournal is an explicit operator action for a new deployment only. Normal
// startup never recreates a missing authority or blesses an unanchored database.
func InitJournal(directory string) error {
	if directory == "" || privateDirectory(directory) != nil {
		return ErrContinuity
	}
	var random [32]byte
	if _, err := rand.Read(random[:]); err != nil {
		return ErrContinuity
	}
	header := journalHeader{1, base64.RawURLEncoding.EncodeToString(random[:])}
	data, err := json.Marshal(header)
	if err != nil {
		return ErrContinuity
	}
	file, err := os.OpenFile(filepath.Join(directory, journalName), os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return ErrContinuity
	}
	defer file.Close()
	if _, err = file.Write(append(data, '\n')); err != nil || file.Sync() != nil || syncDirectory(directory) != nil {
		return ErrContinuity
	}
	return saveAuthorityHead(directory, RecoveryCheckpoint{header.StoreID, 0, Digest(data)})
}

func separateJournal(state, directory string) bool {
	if state == "" || directory == "" {
		return false
	}
	// Resolve ancestors too: a differently spelled/symlinked directory must not
	// turn an ordinary state backup into a backup of its own authority.
	s, err := filepath.EvalSymlinks(state)
	if err != nil {
		return false
	}
	j, err := filepath.EvalSymlinks(directory)
	if err != nil {
		return false
	}
	rel, err := filepath.Rel(s, j)
	return err == nil && rel != "." && (rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)))
}

func openJournal(state, directory string) (*journal, error) {
	if !separateJournal(state, directory) {
		return nil, ErrContinuity
	}
	info, err := os.Lstat(directory)
	if err != nil || !info.IsDir() || info.Mode().Perm() != 0700 || !privateOwner(info) {
		return nil, ErrContinuity
	}
	path := filepath.Join(directory, journalName)
	if privateRegularFile(path) != nil {
		return nil, ErrContinuity
	}
	file, err := os.OpenFile(path, os.O_RDWR|os.O_APPEND, 0)
	if err != nil {
		return nil, ErrContinuity
	}
	lock, err := openCommitLock(filepath.Join(directory, "commit.lock"))
	if err != nil {
		_ = file.Close()
		return nil, ErrContinuity
	}
	j := &journal{file: file, commitLock: lock, gate: make(chan struct{}, 1)}
	return j, nil
}
func (j *journal) close() { _ = j.file.Close(); _ = j.commitLock.Close() }
func (j *journal) locked(ctx context.Context, operation func() error) error {
	select {
	case j.gate <- struct{}{}:
	case <-ctx.Done():
		return ErrContinuity
	}
	defer func() { <-j.gate }()
	if lockCommit(ctx, j.commitLock) != nil {
		return ErrContinuity
	}
	defer unlockCommit(j.commitLock)
	pathInfo, err := os.Lstat(j.file.Name())
	openedInfo, statErr := j.file.Stat()
	if err != nil || statErr != nil || !os.SameFile(pathInfo, openedInfo) || privateRegularFile(j.file.Name()) != nil {
		return ErrContinuity
	}
	return operation()
}

func recordHash(record journalRecord) string {
	record.Hash = ""
	data, _ := json.Marshal(record)
	return Digest(data)
}
func decodeRecord(data []byte) (journalRecord, error) {
	var record journalRecord
	d := json.NewDecoder(bytes.NewReader(data))
	d.DisallowUnknownFields()
	if d.Decode(&record) != nil || d.Decode(new(any)) != io.EOF || record.Sequence < 1 || len(record.Statements) == 0 || recordHash(record) != record.Hash {
		return record, ErrContinuity
	}
	return record, nil
}

// scan streams the bounded records; a partial final write is an error, never an
// excuse to truncate a possibly committed decision. No model/body data enters it.
func (j *journal) scan(start int64, visit func(journalRecord) error) error {
	info, err := j.file.Stat()
	if err != nil || info.Size() < start {
		return ErrContinuity
	}
	reader := bufio.NewReader(io.NewSectionReader(j.file, start, info.Size()-start))
	position := start
	for {
		var line []byte
		for {
			part, err := reader.ReadSlice('\n')
			if err == io.EOF && len(line) == 0 {
				if len(part) == 0 {
					return nil
				}
				return ErrContinuity
			}
			if (err != nil && err != bufio.ErrBufferFull) || len(line)+len(part) > maxJournalRecord+1 {
				return ErrContinuity
			}
			line = append(line, part...)
			if err == nil {
				break
			}
		}
		position += int64(len(line))
		line = line[:len(line)-1]
		if position > info.Size() {
			return ErrContinuity
		}
		if start == 0 && j.offset == 0 {
			var header journalHeader
			d := json.NewDecoder(bytes.NewReader(line))
			d.DisallowUnknownFields()
			if d.Decode(&header) != nil || d.Decode(new(any)) != io.EOF || header.Version != 1 || !validProof(header.StoreID) {
				return ErrContinuity
			}
			canonical, _ := json.Marshal(header)
			if !bytes.Equal(canonical, line) {
				return ErrContinuity
			}
			j.header = header
			j.head = RecoveryCheckpoint{header.StoreID, 0, Digest(line)}
			j.offset = position
			continue
		}
		record, err := decodeRecord(line)
		if err != nil {
			return err
		}
		canonical, _ := json.Marshal(record)
		if !bytes.Equal(canonical, line) {
			return ErrContinuity
		}
		if visit != nil {
			if err := visit(record); err != nil {
				return err
			}
		}
	}
}

func (j *journal) syncTail() error {
	start := j.offset
	err := j.scan(start, func(record journalRecord) error {
		if record.StoreID != j.head.StoreID || record.Sequence != j.head.Sequence+1 || record.Previous != j.head.Hash {
			return ErrContinuity
		}
		encoded, _ := json.Marshal(record)
		j.head = RecoveryCheckpoint{record.StoreID, record.Sequence, record.Hash}
		j.offset += int64(len(encoded)) + 1
		return nil
	})
	if err != nil || j.offset == 0 || !validProof(j.header.StoreID) {
		return ErrContinuity
	}
	return nil
}

func readCheckpoint(ctx context.Context, reader policyReader) (RecoveryCheckpoint, error) {
	var version int
	if reader.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version) != nil {
		return RecoveryCheckpoint{}, ErrContinuity
	}
	if version == 0 {
		var count int
		if reader.QueryRowContext(ctx, "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'").Scan(&count) != nil || count != 0 {
			return RecoveryCheckpoint{}, ErrContinuity
		}
		return RecoveryCheckpoint{}, nil
	}
	if version < 2 || version > 5 {
		return RecoveryCheckpoint{}, ErrContinuity
	}
	var application int
	if reader.QueryRowContext(ctx, "PRAGMA application_id").Scan(&application) != nil || application != 1129465137 {
		return RecoveryCheckpoint{}, ErrContinuity
	}
	var raw []byte
	var checkpoint RecoveryCheckpoint
	if reader.QueryRowContext(ctx, "SELECT value FROM metadata WHERE name='recovery_checkpoint'").Scan(&raw) != nil || json.Unmarshal(raw, &checkpoint) != nil || checkpoint.Sequence < 1 {
		return checkpoint, ErrContinuity
	}
	return checkpoint, nil
}
func putCheckpoint(ctx context.Context, tx *sql.Tx, value RecoveryCheckpoint) error {
	data, err := json.Marshal(value)
	if err != nil {
		return ErrContinuity
	}
	if _, err := tx.ExecContext(ctx, "INSERT INTO metadata(name,value) VALUES('recovery_checkpoint',?) ON CONFLICT(name) DO UPDATE SET value=excluded.value", data); err != nil {
		return ErrContinuity
	}
	return nil
}

func (j *journal) recover(ctx context.Context, db *sql.DB) error {
	if err := j.syncTail(); err != nil {
		return err
	}
	if err := j.checkAuthorityHead(); err != nil {
		return err
	}
	cp, err := readCheckpoint(ctx, db)
	if err != nil {
		return err
	}
	if cp.Sequence == 0 {
		cp = RecoveryCheckpoint{j.header.StoreID, 0, ""}
	}
	if cp.StoreID != j.header.StoreID || cp.Sequence > j.head.Sequence {
		return ErrContinuity
	}
	if cp.Sequence == j.head.Sequence {
		if cp.Sequence > 0 && cp.Hash != j.head.Hash {
			return ErrContinuity
		}
		return nil
	}
	// Validate the checkpoint's position in the whole chain before replaying.
	expected := RecoveryCheckpoint{j.header.StoreID, 0, ""}
	headerData, _ := json.Marshal(j.header)
	expected.Hash = Digest(headerData)
	found := cp.Sequence == 0
	err = j.scan(int64(len(headerData))+1, func(record journalRecord) error {
		if ctx.Err() != nil {
			return ErrContinuity
		}
		if record.StoreID != expected.StoreID || record.Sequence != expected.Sequence+1 || record.Previous != expected.Hash {
			return ErrContinuity
		}
		expected = RecoveryCheckpoint{record.StoreID, record.Sequence, record.Hash}
		if record.Sequence == cp.Sequence {
			if record.Hash != cp.Hash {
				return ErrContinuity
			}
			found = true
		}
		if record.Sequence <= cp.Sequence {
			return nil
		}
		if !found {
			return ErrContinuity
		}
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			return ErrContinuity
		}
		defer tx.Rollback()
		for _, statement := range record.Statements {
			args, err := statement.values()
			if err != nil {
				return err
			}
			if _, err := tx.ExecContext(ctx, statement.SQL, args...); err != nil {
				return ErrContinuity
			}
		}
		if putCheckpoint(ctx, tx, expected) != nil || tx.Commit() != nil {
			return ErrContinuity
		}
		return nil
	})
	if err != nil || !found || expected != j.head {
		return ErrContinuity
	}
	return nil
}

// writeTx deliberately exposes only reads and captured ExecContext, not a raw
// SQL transaction that callers could commit behind the journal's back.
type writeTx struct {
	tx         *sql.Tx
	statements []journalStatement
}

func (t *writeTx) QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row {
	return t.tx.QueryRowContext(ctx, query, args...)
}
func (t *writeTx) QueryContext(ctx context.Context, query string, args ...any) (*sql.Rows, error) {
	return t.tx.QueryContext(ctx, query, args...)
}
func (t *writeTx) ExecContext(ctx context.Context, query string, args ...any) (sql.Result, error) {
	statement := journalStatement{SQL: query, Args: make([]journalValue, len(args))}
	for i, arg := range args {
		value, err := driver.DefaultParameterConverter.ConvertValue(arg)
		if err != nil {
			return nil, ErrStorage
		}
		switch value := value.(type) {
		case nil:
			statement.Args[i] = journalValue{Kind: "null"}
		case string:
			statement.Args[i] = journalValue{Kind: "text", Text: value}
		case int64:
			statement.Args[i] = journalValue{Kind: "integer", Integer: value}
		case bool:
			var n int64
			if value {
				n = 1
			}
			statement.Args[i] = journalValue{Kind: "integer", Integer: n}
		case []byte:
			statement.Args[i] = journalValue{Kind: "blob", Blob: bytes.Clone(value)}
		default:
			return nil, ErrStorage
		}
	}
	result, err := t.tx.ExecContext(ctx, query, args...)
	if err == nil {
		t.statements = append(t.statements, statement)
	}
	return result, err
}
func (s journalStatement) values() ([]any, error) {
	values := make([]any, len(s.Args))
	for i, arg := range s.Args {
		switch arg.Kind {
		case "null":
			values[i] = nil
		case "text":
			values[i] = arg.Text
		case "integer":
			values[i] = arg.Integer
		case "blob":
			values[i] = arg.Blob
		default:
			return nil, ErrContinuity
		}
	}
	return values, nil
}

func (j *journal) transaction(ctx context.Context, db *sql.DB, operation func(*writeTx) error) error {
	return j.locked(ctx, func() error {
		if err := j.recover(ctx, db); err != nil {
			return err
		}
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			return ErrStorage
		}
		defer tx.Rollback()
		write := &writeTx{tx: tx}
		if err := operation(write); err != nil {
			return err
		}
		if len(write.statements) == 0 {
			if tx.Commit() != nil {
				return ErrStorage
			}
			return nil
		}
		record := journalRecord{StoreID: j.head.StoreID, Sequence: j.head.Sequence + 1, Previous: j.head.Hash, Statements: write.statements}
		record.Hash = recordHash(record)
		data, err := json.Marshal(record)
		if err != nil || len(data) > maxJournalRecord {
			return ErrStorage
		}
		// Do not update cached head before fsync. If the result is uncertain, a
		// later syncTail either validates the complete decision or fails closed.
		if _, err := j.file.Write(append(data, '\n')); err != nil || j.file.Sync() != nil {
			return ErrContinuity
		}
		next := RecoveryCheckpoint{record.StoreID, record.Sequence, record.Hash}
		if saveAuthorityHead(filepath.Dir(j.file.Name()), next) != nil {
			return ErrContinuity
		}
		if j.afterAppend != nil {
			if err := j.afterAppend(); err != nil {
				return err
			}
		}
		if putCheckpoint(ctx, tx, next) != nil || tx.Commit() != nil {
			return ErrContinuity
		}
		j.head = next
		j.offset += int64(len(data)) + 1
		return nil
	})
}

// A separately atomically replaced high-water marker detects removal of whole
// valid trailing records even when an older SQLite snapshot is restored too.
// It stays in the trusted authority domain and must never roll back with a DB.
func saveAuthorityHead(directory string, value RecoveryCheckpoint) error {
	data, err := json.Marshal(value)
	if err != nil {
		return ErrContinuity
	}
	file, err := os.CreateTemp(directory, ".head-*")
	if err != nil {
		return ErrContinuity
	}
	name := file.Name()
	defer os.Remove(name)
	defer file.Close()
	if _, err := file.Write(data); err != nil || file.Sync() != nil || file.Close() != nil {
		return ErrContinuity
	}
	if os.Rename(name, filepath.Join(directory, "head.json")) != nil || syncDirectory(directory) != nil {
		return ErrContinuity
	}
	return nil
}
func (j *journal) checkAuthorityHead() error {
	data, err := readPrivateFile(filepath.Join(filepath.Dir(j.file.Name()), "head.json"), 1024)
	if err != nil {
		return ErrContinuity
	}
	var checkpoint RecoveryCheckpoint
	d := json.NewDecoder(bytes.NewReader(data))
	d.DisallowUnknownFields()
	if d.Decode(&checkpoint) != nil || d.Decode(new(any)) != io.EOF || checkpoint.StoreID != j.head.StoreID || checkpoint.Sequence < 0 || checkpoint.Sequence > j.head.Sequence {
		return ErrContinuity
	}
	if checkpoint.Sequence == j.head.Sequence {
		if checkpoint.Hash != j.head.Hash {
			return ErrContinuity
		}
		return nil
	}
	// A complete fsynced append may survive a crash immediately before marker
	// replacement. Validate the old marker in the chain before advancing it.
	header, _ := json.Marshal(j.header)
	found := checkpoint.Sequence == 0 && checkpoint.Hash == Digest(header)
	err = j.scan(int64(len(header))+1, func(record journalRecord) error {
		if record.Sequence == checkpoint.Sequence {
			found = record.Hash == checkpoint.Hash
		}
		return nil
	})
	if err != nil || !found {
		return ErrContinuity
	}
	return saveAuthorityHead(filepath.Dir(j.file.Name()), j.head)
}

func syncDirectory(path string) error {
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	defer file.Close()
	return file.Sync()
}
