package modelconfig

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"strconv"
	"unicode/utf8"
)

type object map[string]any

// Decode strictly decodes, validates, and catalog-checks one canonical model
// configuration. selected is authoritative and must contain unique Agents.
func Decode(data []byte, selected []Agent, catalog []string) (*Config, error) {
	o, err := decodeConfigObject(data)
	if err != nil {
		return nil, err
	}
	return parseConfig(o, selected, catalog, true)
}

// DecodeStructural strictly decodes and validates a canonical model
// configuration without requiring a live catalog. It is used for immutable
// build inputs that are catalog-checked later.
func DecodeStructural(data []byte) (*Config, error) {
	o, err := decodeConfigObject(data)
	if err != nil {
		return nil, err
	}
	selected := make([]Agent, 0, 3)
	for _, agent := range []Agent{Claude, OpenCode, Codex} {
		if _, ok := o[string(agent)]; ok {
			selected = append(selected, agent)
		}
	}
	return parseConfig(o, selected, nil, false)
}

func decodeConfigObject(data []byte) (object, error) {
	if len(data) > MaxConfigSize {
		return nil, invalid("", "size")
	}
	if !utf8.Valid(data) {
		return nil, invalid("", "utf8")
	}
	if !validUnicodeEscapes(data) {
		return nil, invalid("", "unicode")
	}
	root, err := decodeValue(data)
	if err != nil {
		return nil, err
	}
	o, ok := root.(object)
	if !ok {
		return nil, invalid("", "object")
	}
	return o, nil
}

func validUnicodeEscapes(data []byte) bool {
	inString := false
	for i := 0; i < len(data); i++ {
		if data[i] == '"' {
			inString = !inString
			continue
		}
		if !inString || data[i] != '\\' {
			continue
		}
		i++
		if i >= len(data) {
			return false
		}
		if data[i] != 'u' {
			continue
		}
		first, ok := hexQuad(data, i+1)
		if !ok {
			return false
		}
		i += 4
		if first >= 0xd800 && first <= 0xdbff {
			if i+6 >= len(data) || data[i+1] != '\\' || data[i+2] != 'u' {
				return false
			}
			second, ok := hexQuad(data, i+3)
			if !ok || second < 0xdc00 || second > 0xdfff {
				return false
			}
			i += 6
		} else if first >= 0xdc00 && first <= 0xdfff {
			return false
		}
	}
	return true
}

func hexQuad(data []byte, start int) (uint16, bool) {
	if start+4 > len(data) {
		return 0, false
	}
	var value uint16
	for _, c := range data[start : start+4] {
		value <<= 4
		switch {
		case c >= '0' && c <= '9':
			value += uint16(c - '0')
		case c >= 'a' && c <= 'f':
			value += uint16(c-'a') + 10
		case c >= 'A' && c <= 'F':
			value += uint16(c-'A') + 10
		default:
			return 0, false
		}
	}
	return value, true
}

func decodeValue(data []byte) (any, error) {
	d := json.NewDecoder(bytes.NewReader(data))
	d.UseNumber()
	v, err := readValue(d, "", 0)
	if err != nil {
		return nil, err
	}
	if _, err := d.Token(); err != io.EOF {
		if err == nil {
			return nil, invalid("", "trailing_json")
		}
		return nil, invalid("", "json")
	}
	return v, nil
}

func readValue(d *json.Decoder, path string, depth int) (any, error) {
	t, err := d.Token()
	if err != nil {
		return nil, invalid(path, "json")
	}
	switch x := t.(type) {
	case json.Delim:
		switch x {
		case '{':
			o := object{}
			for d.More() {
				kt, err := d.Token()
				if err != nil {
					return nil, invalid(path, "json")
				}
				key, ok := kt.(string)
				if !ok {
					return nil, invalid(path, "json")
				}
				kp := pointer(path, key)
				if _, exists := o[key]; exists {
					return nil, invalid(kp, "duplicate_key")
				}
				v, err := readValue(d, kp, depth+1)
				if err != nil {
					return nil, err
				}
				o[key] = v
			}
			if _, err := d.Token(); err != nil {
				return nil, invalid(path, "json")
			}
			return o, nil
		case '[':
			a := []any{}
			for d.More() {
				if len(a) >= 1024 {
					return nil, invalid(path, "array_size")
				}
				v, err := readValue(d, fmt.Sprintf("%s/%d", path, len(a)), depth+1)
				if err != nil {
					return nil, err
				}
				a = append(a, v)
			}
			if _, err := d.Token(); err != nil {
				return nil, invalid(path, "json")
			}
			return a, nil
		}
	case string:
		if len(x) > 16<<10 {
			return nil, invalid(path, "string_size")
		}
		return x, nil
	case json.Number:
		f, err := strconv.ParseFloat(string(x), 64)
		if err != nil || math.IsInf(f, 0) || math.IsNaN(f) {
			return nil, invalid(path, "number")
		}
		return x, nil
	case bool:
		return x, nil
	case nil:
		return nil, nil
	}
	return nil, invalid(path, "json")
}

func pointer(base, key string) string {
	key = string(bytes.ReplaceAll([]byte(key), []byte("~"), []byte("~0")))
	key = string(bytes.ReplaceAll([]byte(key), []byte("/"), []byte("~1")))
	return base + "/" + key
}
