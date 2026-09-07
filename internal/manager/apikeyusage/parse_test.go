package apikeyusage

import (
	"strings"
	"testing"

	"github.com/codeasier/mtls-router/internal/manager/protocol"
)

func TestParseAcceptsBoundedPerKeyUsage(t *testing.T) {
	snapshot, err := Parse([]byte(`{
		"period":"7d",
		"as_of":"2026-08-28T00:00:00Z",
		"summary":{"requests":12,"prompt_tokens":100,"completion_tokens":20,"cost":1.5},
		"quota":{"used":1.5,"limit":100,"unit":"usd","resets_at":"2026-09-01T00:00:00Z"},
		"by_model":[{"model":"claude-sonnet","requests":12,"prompt_tokens":100,"completion_tokens":20,"cost":1.5}]
	}`), Period7d)
	if err != nil {
		t.Fatalf("Parse() = %v", err)
	}
	if snapshot.Period != Period7d || snapshot.AsOf != "2026-08-28T00:00:00Z" {
		t.Fatalf("meta = %+v", snapshot)
	}
	if snapshot.Summary != (Summary{Requests: 12, PromptTokens: 100, CompletionTokens: 20, Cost: 1.5}) {
		t.Fatalf("summary = %+v", snapshot.Summary)
	}
	if snapshot.Quota == nil || snapshot.Quota.Used != 1.5 || snapshot.Quota.Limit == nil || *snapshot.Quota.Limit != 100 ||
		snapshot.Quota.Unit != QuotaUSD || snapshot.Quota.ResetsAt != "2026-09-01T00:00:00Z" {
		t.Fatalf("quota = %+v", snapshot.Quota)
	}
	if len(snapshot.ByModel) != 1 || snapshot.ByModel[0].Model != "claude-sonnet" {
		t.Fatalf("by_model = %+v", snapshot.ByModel)
	}
}

func TestParseAllowsUnknownSafeFieldsAndUnlimitedQuota(t *testing.T) {
	snapshot, err := Parse([]byte(`{
		"period":"today",
		"extra":"ok",
		"summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0,"note":"ok"},
		"quota":{"used":0,"limit":null,"unit":"tokens"},
		"by_model":[]
	}`), PeriodToday)
	if err != nil {
		t.Fatalf("Parse() = %v", err)
	}
	if snapshot.Quota == nil || snapshot.Quota.Limit != nil || snapshot.Quota.Unit != QuotaTokens {
		t.Fatalf("quota = %+v", snapshot.Quota)
	}
	if snapshot.Quotas != nil {
		t.Fatalf("quotas = %#v", snapshot.Quotas)
	}
	if snapshot.ByModel == nil || len(snapshot.ByModel) != 0 {
		t.Fatalf("by_model = %#v", snapshot.ByModel)
	}
}

func TestParseTreatsMissingOrEmptyQuotasAsAbsent(t *testing.T) {
	for _, body := range []string{
		`{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}`,
		`{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[],"by_model":[]}`,
		`{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":null,"by_model":[]}`,
	} {
		snapshot, err := Parse([]byte(body), Period7d)
		if err != nil {
			t.Fatalf("Parse(%s) = %v", body, err)
		}
		if len(snapshot.Quotas) != 0 {
			t.Fatalf("quotas = %#v", snapshot.Quotas)
		}
	}
}

func TestParseAcceptsProviderQuotasWithoutSynthesizingOverview(t *testing.T) {
	snapshot, err := Parse([]byte(`{
		"period":"1h",
		"summary":{"requests":2,"prompt_tokens":4,"completion_tokens":1,"cost":0.4},
		"quotas":[
			{"provider":"*","period":"week","used":1.25,"limit":10,"unit":"usd","resets_at":"2026-09-14T00:00:00Z"},
			{"provider":"codex","period":"day","used":0.4,"limit":2,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}
		],
		"by_model":[]
	}`), Period1h)
	if err != nil {
		t.Fatalf("Parse() = %v", err)
	}
	if snapshot.Quota != nil {
		t.Fatalf("quota = %+v", snapshot.Quota)
	}
	if len(snapshot.Quotas) != 2 {
		t.Fatalf("quotas = %+v", snapshot.Quotas)
	}
	if snapshot.Quotas[0] != (ProviderQuota{
		Provider: "*", Period: BudgetWeek, Used: 1.25, Limit: 10, Unit: QuotaUSD, ResetsAt: "2026-09-14T00:00:00Z",
	}) {
		t.Fatalf("quotas[0] = %+v", snapshot.Quotas[0])
	}
	if snapshot.Quotas[1].Provider != "codex" || snapshot.Quotas[1].Period != BudgetDay || snapshot.Quotas[1].Limit != 2 {
		t.Fatalf("quotas[1] = %+v", snapshot.Quotas[1])
	}
}

func TestParseAcceptsOverviewQuotaTogetherWithProviderList(t *testing.T) {
	snapshot, err := Parse([]byte(`{
		"period":"this_week",
		"summary":{"requests":1,"prompt_tokens":2,"completion_tokens":3,"cost":1.25},
		"quota":{"used":1.25,"limit":10,"unit":"usd","resets_at":"2026-09-14T00:00:00Z"},
		"quotas":[
			{"provider":"*","period":"week","used":1.25,"limit":10,"unit":"usd","resets_at":"2026-09-14T00:00:00Z"}
		],
		"by_model":[]
	}`), PeriodThisWeek)
	if err != nil {
		t.Fatalf("Parse() = %v", err)
	}
	if snapshot.Quota == nil || snapshot.Quota.Used != 1.25 || snapshot.Quota.Limit == nil || *snapshot.Quota.Limit != 10 {
		t.Fatalf("quota = %+v", snapshot.Quota)
	}
	if len(snapshot.Quotas) != 1 || snapshot.Quotas[0].Provider != "*" {
		t.Fatalf("quotas = %+v", snapshot.Quotas)
	}
}

func TestParseRejectsUnsafeOrUnboundedBodies(t *testing.T) {
	for _, test := range []struct {
		name   string
		body   string
		period Period
	}{
		{name: "period mismatch", body: `{"period":"today","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}`, period: Period7d},
		{name: "missing summary", body: `{"period":"7d","by_model":[]}`, period: Period7d},
		{name: "missing by_model", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0}}`, period: Period7d},
		{name: "negative requests", body: `{"period":"7d","summary":{"requests":-1,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}`, period: Period7d},
		{name: "fractional tokens", body: `{"period":"7d","summary":{"requests":1,"prompt_tokens":1.5,"completion_tokens":0,"cost":0},"by_model":[]}`, period: Period7d},
		{name: "api_key field", body: `{"period":"7d","api_key":"secret","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}`, period: Period7d},
		{name: "token field", body: `{"period":"7d","token":"secret","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[]}`, period: Period7d},
		{name: "too many models", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[` + strings.Repeat(`{"model":"m","requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},`, 65) + `{"model":"z","requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0}]}`, period: Period7d},
		{name: "invalid model id", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"by_model":[{"model":" m","requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0}]}`, period: Period7d},
		{name: "not object", body: `[]`, period: Period7d},
		{name: "quota bad unit", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quota":{"used":0,"unit":"hours"},"by_model":[]}`, period: Period7d},
		{name: "quotas not array", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":{},"by_model":[]}`, period: Period7d},
		{name: "quotas unit tokens", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"day","used":0,"limit":1,"unit":"tokens","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}`, period: Period7d},
		{name: "quotas null limit", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"day","used":0,"limit":null,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}`, period: Period7d},
		{name: "quotas zero limit", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"day","used":0,"limit":0,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}`, period: Period7d},
		{name: "quotas usage period token", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"today","used":0,"limit":1,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}`, period: Period7d},
		{name: "quotas blank provider", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"","period":"day","used":0,"limit":1,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}`, period: Period7d},
		{name: "quotas missing resets", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[{"provider":"*","period":"day","used":0,"limit":1,"unit":"usd"}],"by_model":[]}`, period: Period7d},
		{name: "too many quotas", body: `{"period":"7d","summary":{"requests":0,"prompt_tokens":0,"completion_tokens":0,"cost":0},"quotas":[` + strings.Repeat(`{"provider":"p","period":"day","used":0,"limit":1,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"},`, 32) + `{"provider":"z","period":"day","used":0,"limit":1,"unit":"usd","resets_at":"2026-09-08T00:00:00Z"}],"by_model":[]}`, period: Period7d},
	} {
		t.Run(test.name, func(t *testing.T) {
			_, err := Parse([]byte(test.body), test.period)
			if CodeOf(err) != protocol.CodeUsageResponseInvalid {
				t.Fatalf("Parse() = %v, want USAGE_RESPONSE_INVALID", err)
			}
			if err != nil && strings.Contains(err.Error(), "secret") {
				t.Fatalf("error leaked secret: %v", err)
			}
		})
	}
}

func TestNormalizePeriod(t *testing.T) {
	period, err := NormalizePeriod("")
	if err != nil || period != Period7d {
		t.Fatalf("default = %q, %v", period, err)
	}
	for _, want := range []Period{Period1h, Period12h, Period24h, Period7d, Period30d, PeriodToday, PeriodThisWeek, PeriodThisMonth} {
		got, err := NormalizePeriod(string(want))
		if err != nil || got != want {
			t.Fatalf("NormalizePeriod(%q) = %q, %v", want, got, err)
		}
	}
	for _, invalid := range []string{"week", "month", "1d"} {
		if _, err := NormalizePeriod(invalid); CodeOf(err) != protocol.CodeInvalidParams {
			t.Fatalf("invalid period %q code = %v", invalid, err)
		}
	}
}
