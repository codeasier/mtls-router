package modelcatalog

import "testing"

func TestParseSimplifyDefault(t *testing.T) {
	if Simplify != "True" {
		t.Fatalf("Simplify = %q, want %q", Simplify, "True")
	}

	got, err := ParseSimplify()
	if err != nil {
		t.Fatalf("ParseSimplify() error = %v", err)
	}
	if !got {
		t.Fatal("ParseSimplify() = false, want true")
	}
}

func TestParseSimplifyAcceptsCaseInsensitiveBooleans(t *testing.T) {
	tests := []struct {
		value string
		want  bool
	}{
		{value: "true", want: true},
		{value: "TRUE", want: true},
		{value: "TrUe", want: true},
		{value: "false", want: false},
		{value: "FALSE", want: false},
		{value: "FaLsE", want: false},
	}

	for _, test := range tests {
		t.Run(test.value, func(t *testing.T) {
			setSimplifyForTest(t, test.value)

			got, err := ParseSimplify()
			if err != nil {
				t.Fatalf("ParseSimplify() error = %v", err)
			}
			if got != test.want {
				t.Fatalf("ParseSimplify() = %t, want %t", got, test.want)
			}
		})
	}
}

func TestParseSimplifyRejectsInvalidValues(t *testing.T) {
	for _, value := range []string{
		"",
		" ",
		"\t",
		" true",
		"false ",
		"1",
		"0",
		"yes",
		"no",
		"falſe",
		"truK",
		"enabled-secret",
	} {
		t.Run(value, func(t *testing.T) {
			setSimplifyForTest(t, value)

			_, err := ParseSimplify()
			if err == nil || err.Error() != "invalid embedded simplify value" {
				t.Fatalf("ParseSimplify() error = %v, want %q", err, "invalid embedded simplify value")
			}
		})
	}
}

func setSimplifyForTest(t *testing.T, value string) {
	t.Helper()
	previous := Simplify
	Simplify = value
	t.Cleanup(func() {
		Simplify = previous
	})
}
