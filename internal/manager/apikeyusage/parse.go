package apikeyusage

import (
	"bytes"
	"encoding/json"
	"io"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/codeasier/mtls-router/internal/manager/protocol"
)

const (
	maxModels  = 64
	maxIDBytes = 256
)

// Period is the requested usage window.
type Period string

const (
	PeriodToday Period = "today"
	Period7d    Period = "7d"
	Period30d   Period = "30d"
)

// QuotaUnit is the optional remaining-budget unit.
type QuotaUnit string

const (
	QuotaUSD      QuotaUnit = "usd"
	QuotaTokens   QuotaUnit = "tokens"
	QuotaRequests QuotaUnit = "requests"
)

// Snapshot is one per-key usage document after fail-closed validation.
type Snapshot struct {
	Period  Period
	AsOf    string
	Summary Summary
	Quota   *Quota
	ByModel []Model
}

// Summary is the bounded numeric totals for one period.
type Summary struct {
	Requests         int64
	PromptTokens     int64
	CompletionTokens int64
	Cost             float64
}

// Quota is an optional remaining-budget projection for the same key.
type Quota struct {
	Used     float64
	Limit    *float64
	Unit     QuotaUnit
	ResetsAt string
}

// Model is one per-model row belonging to the authenticated key.
type Model struct {
	Model            string
	Requests         int64
	PromptTokens     int64
	CompletionTokens int64
	Cost             float64
}

// NormalizePeriod accepts the protocol period or the default 7d window.
func NormalizePeriod(value string) (Period, error) {
	switch strings.TrimSpace(value) {
	case "", string(Period7d):
		return Period7d, nil
	case string(PeriodToday):
		return PeriodToday, nil
	case string(Period30d):
		return Period30d, nil
	default:
		return "", &Error{Code: protocol.CodeInvalidParams, msg: "usage period is invalid"}
	}
}

// Parse validates one bounded /v1/usage body for the requested period.
func Parse(body []byte, period Period) (Snapshot, error) {
	if !utf8.Valid(body) {
		return Snapshot{}, responseInvalid()
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return Snapshot{}, responseInvalid()
	}
	var snapshot Snapshot
	seen := map[string]bool{}
	for decoder.More() {
		keyToken, err := decoder.Token()
		key, ok := keyToken.(string)
		if err != nil || !ok || !seenField(seen, key) {
			return Snapshot{}, responseInvalid()
		}
		switch key {
		case "period":
			value, err := decodeString(decoder)
			if err != nil || Period(value) != period {
				return Snapshot{}, responseInvalid()
			}
			snapshot.Period = period
		case "as_of":
			value, err := decodeString(decoder)
			if err != nil || !validTime(value) {
				return Snapshot{}, responseInvalid()
			}
			snapshot.AsOf = value
		case "summary":
			summary, err := parseSummary(decoder)
			if err != nil {
				return Snapshot{}, err
			}
			snapshot.Summary = summary
		case "quota":
			quota, err := parseQuota(decoder)
			if err != nil {
				return Snapshot{}, err
			}
			snapshot.Quota = quota
		case "by_model":
			models, err := parseModels(decoder)
			if err != nil {
				return Snapshot{}, err
			}
			snapshot.ByModel = models
		default:
			if err := skipValue(decoder); err != nil {
				return Snapshot{}, responseInvalid()
			}
		}
	}
	if _, err := decoder.Token(); err != nil || !seen["period"] || !seen["summary"] {
		return Snapshot{}, responseInvalid()
	}
	if token, err := decoder.Token(); err != io.EOF || token != nil {
		return Snapshot{}, responseInvalid()
	}
	if snapshot.ByModel == nil {
		snapshot.ByModel = []Model{}
	}
	return snapshot, nil
}

func parseSummary(decoder *json.Decoder) (Summary, error) {
	fields, err := decodeObject(decoder)
	if err != nil {
		return Summary{}, err
	}
	requests, err := requiredCount(fields, "requests")
	if err != nil {
		return Summary{}, err
	}
	prompt, err := requiredCount(fields, "prompt_tokens")
	if err != nil {
		return Summary{}, err
	}
	completion, err := requiredCount(fields, "completion_tokens")
	if err != nil {
		return Summary{}, err
	}
	cost, err := requiredAmount(fields, "cost")
	if err != nil {
		return Summary{}, err
	}
	return Summary{Requests: requests, PromptTokens: prompt, CompletionTokens: completion, Cost: cost}, nil
}

func parseQuota(decoder *json.Decoder) (*Quota, error) {
	if decoder == nil {
		return nil, responseInvalid()
	}
	token, err := decoder.Token()
	if err != nil {
		return nil, responseInvalid()
	}
	if token == nil {
		return nil, nil
	}
	if token != json.Delim('{') {
		return nil, responseInvalid()
	}
	fields := map[string]json.Token{}
	seen := map[string]bool{}
	for decoder.More() {
		keyToken, err := decoder.Token()
		key, ok := keyToken.(string)
		if err != nil || !ok || !seenField(seen, key) {
			return nil, responseInvalid()
		}
		value, err := decoder.Token()
		if err != nil {
			return nil, responseInvalid()
		}
		if _, isDelim := value.(json.Delim); isDelim {
			return nil, responseInvalid()
		}
		fields[key] = value
	}
	if token, err := decoder.Token(); err != nil || token != json.Delim('}') {
		return nil, responseInvalid()
	}
	used, err := requiredAmount(fields, "used")
	if err != nil {
		return nil, err
	}
	unitValue, ok := fields["unit"].(string)
	if !ok {
		return nil, responseInvalid()
	}
	unit := QuotaUnit(unitValue)
	if unit != QuotaUSD && unit != QuotaTokens && unit != QuotaRequests {
		return nil, responseInvalid()
	}
	quota := &Quota{Used: used, Unit: unit}
	if limitToken, present := fields["limit"]; present && limitToken != nil {
		amount, err := decodeAmount(limitToken)
		if err != nil {
			return nil, err
		}
		quota.Limit = &amount
	}
	if resets, present := fields["resets_at"]; present && resets != nil {
		value, ok := resets.(string)
		if !ok || !validTime(value) {
			return nil, responseInvalid()
		}
		quota.ResetsAt = value
	}
	return quota, nil
}

func parseModels(decoder *json.Decoder) ([]Model, error) {
	token, err := decoder.Token()
	if err != nil || token != json.Delim('[') {
		return nil, responseInvalid()
	}
	models := make([]Model, 0)
	for decoder.More() {
		fields, err := decodeObject(decoder)
		if err != nil {
			return nil, err
		}
		id, ok := fields["model"].(string)
		if !ok || !validID(id) {
			return nil, responseInvalid()
		}
		requests, err := requiredCount(fields, "requests")
		if err != nil {
			return nil, err
		}
		prompt, err := requiredCount(fields, "prompt_tokens")
		if err != nil {
			return nil, err
		}
		completion, err := requiredCount(fields, "completion_tokens")
		if err != nil {
			return nil, err
		}
		cost, err := requiredAmount(fields, "cost")
		if err != nil {
			return nil, err
		}
		models = append(models, Model{
			Model: id, Requests: requests, PromptTokens: prompt, CompletionTokens: completion, Cost: cost,
		})
		if len(models) > maxModels {
			return nil, responseInvalid()
		}
	}
	if token, err := decoder.Token(); err != nil || token != json.Delim(']') {
		return nil, responseInvalid()
	}
	return models, nil
}

func decodeObject(decoder *json.Decoder) (map[string]json.Token, error) {
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return nil, responseInvalid()
	}
	fields := map[string]json.Token{}
	seen := map[string]bool{}
	for decoder.More() {
		keyToken, err := decoder.Token()
		key, ok := keyToken.(string)
		if err != nil || !ok || !seenField(seen, key) {
			return nil, responseInvalid()
		}
		value, err := decoder.Token()
		if err != nil {
			return nil, responseInvalid()
		}
		if _, isDelim := value.(json.Delim); isDelim {
			return nil, responseInvalid()
		}
		fields[key] = value
	}
	if token, err := decoder.Token(); err != nil || token != json.Delim('}') {
		return nil, responseInvalid()
	}
	return fields, nil
}

func seenField(seen map[string]bool, key string) bool {
	if key == "" || sensitiveField(key) || seen[key] {
		return false
	}
	seen[key] = true
	return true
}

func sensitiveField(name string) bool {
	normalized := strings.ToLower(strings.NewReplacer("_", "", "-", "").Replace(name))
	switch normalized {
	case "apikey", "key", "token", "authorization", "secret", "password", "credential", "bearer", "accesstoken", "clientsecret":
		return true
	default:
		return false
	}
}

func decodeString(decoder *json.Decoder) (string, error) {
	token, err := decoder.Token()
	value, ok := token.(string)
	if err != nil || !ok {
		return "", responseInvalid()
	}
	return value, nil
}

func requiredCount(fields map[string]json.Token, key string) (int64, error) {
	value, ok := fields[key]
	if !ok {
		return 0, responseInvalid()
	}
	number, ok := value.(json.Number)
	if !ok {
		return 0, responseInvalid()
	}
	if strings.ContainsAny(number.String(), ".eE") {
		return 0, responseInvalid()
	}
	count, err := number.Int64()
	if err != nil || count < 0 {
		return 0, responseInvalid()
	}
	return count, nil
}

func requiredAmount(fields map[string]json.Token, key string) (float64, error) {
	value, ok := fields[key]
	if !ok {
		return 0, responseInvalid()
	}
	return decodeAmount(value)
}

func decodeAmount(value json.Token) (float64, error) {
	number, ok := value.(json.Number)
	if !ok {
		return 0, responseInvalid()
	}
	amount, err := number.Float64()
	if err != nil || amount < 0 || strings.EqualFold(number.String(), "nan") ||
		strings.EqualFold(number.String(), "inf") || strings.EqualFold(number.String(), "+inf") ||
		strings.EqualFold(number.String(), "-inf") {
		return 0, responseInvalid()
	}
	if _, err := strconv.ParseFloat(number.String(), 64); err != nil {
		return 0, responseInvalid()
	}
	return amount, nil
}

func validTime(value string) bool {
	if value == "" {
		return false
	}
	_, err := time.Parse(time.RFC3339, value)
	return err == nil
}

func validID(id string) bool {
	if id == "" || len(id) > maxIDBytes || !utf8.ValidString(id) {
		return false
	}
	first, _ := utf8.DecodeRuneInString(id)
	last, _ := utf8.DecodeLastRuneInString(id)
	if unicode.IsSpace(first) || unicode.IsSpace(last) {
		return false
	}
	for _, r := range id {
		if unicode.IsControl(r) {
			return false
		}
	}
	return true
}

func skipValue(decoder *json.Decoder) error {
	token, err := decoder.Token()
	if err != nil {
		return err
	}
	delim, ok := token.(json.Delim)
	if !ok {
		return nil
	}
	switch delim {
	case '{':
		for decoder.More() {
			if _, err := decoder.Token(); err != nil {
				return err
			}
			if err := skipValue(decoder); err != nil {
				return err
			}
		}
		_, err = decoder.Token()
		return err
	case '[':
		for decoder.More() {
			if err := skipValue(decoder); err != nil {
				return err
			}
		}
		_, err = decoder.Token()
		return err
	default:
		return responseInvalid()
	}
}
