package modelcatalog

import (
	"fmt"
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/protocol"
)

func TestParseNormalizesCatalog(t *testing.T) {
	body := []byte(`{
  "ignored":{"nested":[1,true,null,{"value":"unused"}]},
  "data":[
    {"id":"z-model","owned_by":"service"},
    {"created":1,"id":"é-model"},
    {"id":"a-model"},
    {"id":"z-model"}
  ]
}`)
	models, err := Parse(body, false)
	if err != nil {
		t.Fatalf("Parse() error = %v", err)
	}
	want := []string{"a-model", "z-model", "é-model"}
	if fmt.Sprint(models) != fmt.Sprint(want) {
		t.Fatalf("Parse() = %q, want %q", models, want)
	}
}

func TestParseRejectsInvalidResponses(t *testing.T) {
	tests := []struct {
		name string
		body []byte
	}{
		{name: "empty", body: nil},
		{name: "invalid UTF-8", body: []byte{'{', '"', 'd', 'a', 't', 'a', '"', ':', '[', '{', '"', 'i', 'd', '"', ':', '"', 0xff, '"', '}', ']', '}'}},
		{name: "root null", body: []byte(`null`)},
		{name: "root array", body: []byte(`[]`)},
		{name: "missing data", body: []byte(`{}`)},
		{name: "duplicate data", body: []byte(`{"data":[],"data":[]}`)},
		{name: "data null", body: []byte(`{"data":null}`)},
		{name: "data object", body: []byte(`{"data":{}}`)},
		{name: "item scalar", body: []byte(`{"data":["model"]}`)},
		{name: "item array", body: []byte(`{"data":[[]]}`)},
		{name: "missing id", body: []byte(`{"data":[{}]}`)},
		{name: "duplicate id", body: []byte(`{"data":[{"id":"a","id":"b"}]}`)},
		{name: "id null", body: []byte(`{"data":[{"id":null}]}`)},
		{name: "id number", body: []byte(`{"data":[{"id":1}]}`)},
		{name: "empty id", body: []byte(`{"data":[{"id":""}]}`)},
		{name: "leading ASCII whitespace", body: []byte(`{"data":[{"id":" model"}]}`)},
		{name: "trailing Unicode whitespace", body: []byte("{\"data\":[{\"id\":\"model\u3000\"}]}")},
		{name: "leading non-breaking space", body: []byte("{\"data\":[{\"id\":\"\u00a0model\"}]}")},
		{name: "ASCII control", body: []byte(`{"data":[{"id":"model\u0000x"}]}`)},
		{name: "Unicode control", body: []byte(`{"data":[{"id":"model\u0085x"}]}`)},
		{name: "257 byte id", body: []byte(`{"data":[{"id":"` + strings.Repeat("a", 257) + `"}]}`)},
		{name: "malformed", body: []byte(`{"data":[`)},
		{name: "trailing object", body: []byte(`{"data":[{"id":"a"}]} {}`)},
		{name: "trailing scalar", body: []byte(`{"data":[{"id":"a"}]} true`)},
		{name: "malformed ignored field", body: []byte(`{"ignored":[},"data":[{"id":"a"}]}`)},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			_, err := Parse(test.body, false)
			assertCode(t, err, protocol.CodeModelResponseInvalid)
		})
	}
}

func TestParseIDBoundaries(t *testing.T) {
	for _, id := range []string{
		strings.Repeat("a", 256),
		"model with interior space",
		"模型-é",
		"model\u200bzero-width-space",
	} {
		body := []byte(fmt.Sprintf(`{"data":[{"id":%q}]}`, id))
		models, err := Parse(body, false)
		if err != nil {
			t.Fatalf("Parse(%q) error = %v", id, err)
		}
		if len(models) != 1 || models[0] != id {
			t.Fatalf("Parse(%q) = %q", id, models)
		}
	}
}

func TestParseCatalogCountBoundaries(t *testing.T) {
	var body strings.Builder
	body.WriteString(`{"data":[`)
	for i := range maxModels {
		if i > 0 {
			body.WriteByte(',')
		}
		fmt.Fprintf(&body, `{"id":"model-%04d"}`, i)
	}
	body.WriteString(`]}`)
	models, err := Parse([]byte(body.String()), false)
	if err != nil || len(models) != maxModels {
		t.Fatalf("1000 models: len = %d, err = %v", len(models), err)
	}

	body.Reset()
	body.WriteString(`{"data":[`)
	for i := range maxModels + 1 {
		if i > 0 {
			body.WriteByte(',')
		}
		fmt.Fprintf(&body, `{"id":"model-%04d"}`, i)
	}
	body.WriteString(`]}`)
	_, err = Parse([]byte(body.String()), false)
	assertCode(t, err, protocol.CodeModelResponseInvalid)

	duplicates := `{"data":[` + strings.Repeat(`{"id":"same"},`, maxModels+10) + `{"id":"same"}]}`
	models, err = Parse([]byte(duplicates), false)
	if err != nil || len(models) != 1 || models[0] != "same" {
		t.Fatalf("duplicate models = %q, err = %v", models, err)
	}
}

func TestParseEmptyCatalog(t *testing.T) {
	_, err := Parse([]byte(`{"data":[]}`), false)
	assertCode(t, err, protocol.CodeModelCatalogEmpty)
}

func TestParseSimplifiesCatalog(t *testing.T) {
	body := []byte(`{"data":[
  {"id":"provider/model"},
  {"id":"plain"},
  {"id":"wide／slash"},
  {"id":"back\\slash"},
  {"id":"provider/nested/model"},
  {"id":"plain"}
]}`)

	tests := []struct {
		name     string
		simplify bool
		want     []string
	}{
		{
			name:     "disabled",
			simplify: false,
			want:     []string{"back\\slash", "plain", "provider/model", "provider/nested/model", "wide／slash"},
		},
		{
			name:     "enabled",
			simplify: true,
			want:     []string{"back\\slash", "plain", "wide／slash"},
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			models, err := Parse(body, test.simplify)
			if err != nil {
				t.Fatalf("Parse() error = %v", err)
			}
			if fmt.Sprint(models) != fmt.Sprint(test.want) {
				t.Fatalf("Parse() = %q, want %q", models, test.want)
			}
			if test.simplify && cap(models) != len(models) {
				t.Fatalf("Parse() capacity = %d, want %d; backing tail must not be exposed", cap(models), len(models))
			}
		})
	}
}

func TestParseSimplificationPreservesValidationAndLimits(t *testing.T) {
	t.Run("filtered to empty", func(t *testing.T) {
		_, err := Parse([]byte(`{"data":[{"id":"provider/model"}]}`), true)
		assertCode(t, err, protocol.CodeModelCatalogEmpty)
	})

	t.Run("malformed filtered model", func(t *testing.T) {
		_, err := Parse([]byte(`{"data":[{"id":"valid"},{"id":"provider/model\u0000"}]}`), true)
		assertCode(t, err, protocol.CodeModelResponseInvalid)
	})

	t.Run("model limit before filtering", func(t *testing.T) {
		var body strings.Builder
		body.WriteString(`{"data":[`)
		for i := range maxModels + 1 {
			if i > 0 {
				body.WriteByte(',')
			}
			fmt.Fprintf(&body, `{"id":"provider/model-%04d"}`, i)
		}
		body.WriteString(`]}`)

		_, err := Parse([]byte(body.String()), true)
		assertCode(t, err, protocol.CodeModelResponseInvalid)
	})
}

func assertCode(t *testing.T, err error, want protocol.ErrorCode) {
	t.Helper()
	if got := CodeOf(err); got != want {
		t.Fatalf("CodeOf(%v) = %q, want %q", err, got, want)
	}
}
