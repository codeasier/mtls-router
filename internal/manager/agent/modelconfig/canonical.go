package modelconfig

import (
	"bytes"
	"encoding/json"
	"math"
	"reflect"
	"sort"
	"strconv"
	"unicode/utf16"
)

// Canonical returns RFC 8785 JSON Canonicalization Scheme bytes.
func Canonical(config *Config) ([]byte, error) {
	plain, err := json.Marshal(config)
	if err != nil {
		return nil, err
	}
	v, err := decodeValue(plain)
	if err != nil {
		return nil, err
	}
	return marshalJCS(v)
}

// CanonicalValue encodes an already validated internal value using the same
// RFC 8785 encoder as canonical model configuration. Callers must not use it
// as a substitute for schema validation.
func CanonicalValue(value any) ([]byte, error) {
	return marshalJCS(value)
}

func marshalJCS(v any) ([]byte, error) {
	var out bytes.Buffer
	if err := appendJCS(&out, v); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}

func appendJCS(out *bytes.Buffer, v any) error {
	switch x := v.(type) {
	case nil:
		out.WriteString("null")
	case bool:
		out.WriteString(strconv.FormatBool(x))
	case string:
		appendString(out, x)
	case json.Number:
		f, err := x.Float64()
		if err != nil || math.IsInf(f, 0) || math.IsNaN(f) {
			return invalid("", "number")
		}
		out.WriteString(formatNumber(f))
	case float64:
		if math.IsInf(x, 0) || math.IsNaN(x) {
			return invalid("", "number")
		}
		out.WriteString(formatNumber(x))
	case int:
		out.WriteString(strconv.Itoa(x))
	case int64:
		out.WriteString(strconv.FormatInt(x, 10))
	case []any:
		out.WriteByte('[')
		for i, item := range x {
			if i > 0 {
				out.WriteByte(',')
			}
			if err := appendJCS(out, item); err != nil {
				return err
			}
		}
		out.WriteByte(']')
	case object:
		keys := make([]string, 0, len(x))
		for key := range x {
			keys = append(keys, key)
		}
		sort.Slice(keys, func(i, j int) bool { return utf16Less(keys[i], keys[j]) })
		out.WriteByte('{')
		for i, key := range keys {
			if i > 0 {
				out.WriteByte(',')
			}
			appendString(out, key)
			out.WriteByte(':')
			if err := appendJCS(out, x[key]); err != nil {
				return err
			}
		}
		out.WriteByte('}')
	case map[string]any:
		return appendJCS(out, object(x))
	default:
		data, err := json.Marshal(v)
		if err != nil {
			return err
		}
		decoded, err := decodeValue(data)
		if err != nil {
			return err
		}
		// Stop self-round-tripping custom values from recursing forever; they
		// are not one of the JSON value types accepted above.
		if reflect.DeepEqual(decoded, v) {
			return invalid("", "json_value")
		}
		return appendJCS(out, decoded)
	}
	return nil
}

func formatNumber(f float64) string {
	if f == 0 {
		return "0"
	}
	a := math.Abs(f)
	if a >= 1e21 || a < 1e-6 {
		s := strconv.FormatFloat(f, 'e', -1, 64)
		i := bytes.LastIndexByte([]byte(s), 'e')
		mantissa, exponent := s[:i], s[i+1:]
		sign := ""
		if exponent[0] == '+' || exponent[0] == '-' {
			sign, exponent = exponent[:1], exponent[1:]
		}
		for len(exponent) > 1 && exponent[0] == '0' {
			exponent = exponent[1:]
		}
		return mantissa + "e" + sign + exponent
	}
	return strconv.FormatFloat(f, 'f', -1, 64)
}

func appendString(out *bytes.Buffer, s string) {
	out.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"', '\\':
			out.WriteByte('\\')
			out.WriteRune(r)
		case '\b':
			out.WriteString(`\b`)
		case '\t':
			out.WriteString(`\t`)
		case '\n':
			out.WriteString(`\n`)
		case '\f':
			out.WriteString(`\f`)
		case '\r':
			out.WriteString(`\r`)
		default:
			if r < 0x20 {
				out.WriteString(`\u00`)
				out.WriteString(strconv.FormatInt(int64(r)>>4, 16))
				out.WriteString(strconv.FormatInt(int64(r)&15, 16))
			} else {
				out.WriteRune(r)
			}
		}
	}
	out.WriteByte('"')
}

func utf16Less(a, b string) bool {
	aa, bb := utf16.Encode([]rune(a)), utf16.Encode([]rune(b))
	for i := 0; i < len(aa) && i < len(bb); i++ {
		if aa[i] != bb[i] {
			return aa[i] < bb[i]
		}
	}
	return len(aa) < len(bb)
}
