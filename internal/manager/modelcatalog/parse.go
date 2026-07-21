package modelcatalog

import (
	"bytes"
	"encoding/json"
	"io"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"
)

const (
	maxIDBytes = 256
	maxModels  = 1000
)

// Parse validates and normalizes one bounded /v1/models response body. After
// full validation, simplify excludes IDs containing ASCII '/'.
func Parse(body []byte, simplify bool) ([]string, error) {
	if !utf8.Valid(body) {
		return nil, responseInvalid()
	}
	decoder := json.NewDecoder(bytes.NewReader(body))

	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return nil, responseInvalid()
	}
	seenData := false
	models := make(map[string]struct{})
	for decoder.More() {
		keyToken, err := decoder.Token()
		key, ok := keyToken.(string)
		if err != nil || !ok {
			return nil, responseInvalid()
		}
		if key != "data" {
			if err := skipValue(decoder); err != nil {
				return nil, responseInvalid()
			}
			continue
		}
		if seenData {
			return nil, responseInvalid()
		}
		seenData = true
		if err := parseData(decoder, models); err != nil {
			return nil, responseInvalid()
		}
	}
	if _, err := decoder.Token(); err != nil || !seenData {
		return nil, responseInvalid()
	}
	if token, err := decoder.Token(); err != io.EOF || token != nil {
		return nil, responseInvalid()
	}
	result := make([]string, 0, len(models))
	for model := range models {
		result = append(result, model)
	}
	// Go string comparison is lexicographic over the underlying UTF-8 bytes.
	sort.Strings(result)
	if simplify {
		filtered := make([]string, 0, len(result))
		for _, model := range result {
			if !strings.Contains(model, "/") {
				filtered = append(filtered, model)
			}
		}
		result = filtered[:len(filtered):len(filtered)]
	}
	if len(result) == 0 {
		return nil, catalogEmpty()
	}
	return result, nil
}

func parseData(decoder *json.Decoder, models map[string]struct{}) error {
	token, err := decoder.Token()
	if err != nil || token != json.Delim('[') {
		return responseInvalid()
	}
	for decoder.More() {
		id, err := parseItem(decoder)
		if err != nil {
			return err
		}
		models[id] = struct{}{}
		if len(models) > maxModels {
			return responseInvalid()
		}
	}
	if token, err := decoder.Token(); err != nil || token != json.Delim(']') {
		return responseInvalid()
	}
	return nil
}

func parseItem(decoder *json.Decoder) (string, error) {
	token, err := decoder.Token()
	if err != nil || token != json.Delim('{') {
		return "", responseInvalid()
	}
	seenID := false
	id := ""
	for decoder.More() {
		keyToken, err := decoder.Token()
		key, ok := keyToken.(string)
		if err != nil || !ok {
			return "", responseInvalid()
		}
		if key != "id" {
			if err := skipValue(decoder); err != nil {
				return "", responseInvalid()
			}
			continue
		}
		if seenID {
			return "", responseInvalid()
		}
		seenID = true
		value, err := decoder.Token()
		if err != nil {
			return "", responseInvalid()
		}
		var stringValue bool
		id, stringValue = value.(string)
		if !stringValue || !validID(id) {
			return "", responseInvalid()
		}
	}
	if token, err := decoder.Token(); err != nil || token != json.Delim('}') || !seenID {
		return "", responseInvalid()
	}
	return id, nil
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
