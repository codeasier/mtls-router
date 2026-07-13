package protocol

import (
	"bytes"
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestDeadlinesCoverEveryMethod(t *testing.T) {
	want := map[Method]time.Duration{
		MethodManagerInfo: time.Second, MethodDiagnosticsCollect: 5 * time.Second,
		MethodRouterStatus: time.Second, MethodRouterStart: 20 * time.Second,
		MethodRouterStop: 7 * time.Second, MethodRouterHealth: 5 * time.Second,
		MethodRouterVersion: time.Second, MethodRouterLogs: 2 * time.Second,
		MethodAgentDetect: 5 * time.Second, MethodAgentPreview: 5 * time.Second,
		MethodAgentWrite: 30 * time.Second,
	}
	got := Deadlines()
	if len(got) != len(want) {
		t.Fatalf("deadline count = %d, want %d", len(got), len(want))
	}
	for method, duration := range want {
		if got[method] != duration {
			t.Errorf("deadline for %q = %s, want %s", method, got[method], duration)
		}
	}
}

func TestServeCorrelatesSequentialRequestsAndKeepsStdoutPure(t *testing.T) {
	var calls []string
	server := NewServer(map[Method]Handler{
		MethodManagerInfo: func(_ context.Context, params json.RawMessage) (any, *Error) {
			calls = append(calls, string(params))
			return ManagerInfoResult{Version: "v1", Target: "test"}, nil
		},
	})
	input := strings.NewReader("{\"id\":\"first\",\"method\":\"manager.info\"}\n{\"id\":\"second\",\"method\":\"manager.info\",\"params\":{}}\n")
	var output bytes.Buffer
	if err := server.Serve(context.Background(), input, &output); err != nil {
		t.Fatal(err)
	}
	if len(calls) != 2 || calls[0] != "{}" || calls[1] != "{}" {
		t.Fatalf("calls = %#v", calls)
	}

	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if len(lines) != 2 {
		t.Fatalf("response lines = %d, output %q", len(lines), output.String())
	}
	for index, line := range lines {
		var response Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatalf("response %d is not JSON: %v", index, err)
		}
		wantID := []string{"first", "second"}[index]
		if response.ID == nil || *response.ID != wantID || response.Error != nil {
			t.Fatalf("response %d = %#v", index, response)
		}
	}
}

func TestServeMalformedAndMissingIDsUseNullID(t *testing.T) {
	server := NewServer(nil)
	input := strings.NewReader("not-json\n{\"method\":\"manager.info\"}\n{\"id\":42,\"method\":\"manager.info\"}\n")
	var output bytes.Buffer
	if err := server.Serve(context.Background(), input, &output); err != nil {
		t.Fatal(err)
	}
	for _, line := range strings.Split(strings.TrimSpace(output.String()), "\n") {
		var response Response
		if err := json.Unmarshal([]byte(line), &response); err != nil {
			t.Fatal(err)
		}
		if response.ID != nil || response.Error == nil || response.Error.Code != CodeInvalidRequest {
			t.Fatalf("response = %#v", response)
		}
	}
}

func TestServeUnknownMethodPreservesValidID(t *testing.T) {
	server := NewServer(nil)
	var output bytes.Buffer
	if err := server.Serve(context.Background(), strings.NewReader("{\"id\":\"request-1\",\"method\":\"other\"}\n"), &output); err != nil {
		t.Fatal(err)
	}
	var response Response
	if err := json.Unmarshal(output.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	if response.ID == nil || *response.ID != "request-1" || response.Error == nil || response.Error.Code != CodeUnknownMethod {
		t.Fatalf("response = %#v", response)
	}
}

func TestServeReturnsStableHandlerError(t *testing.T) {
	server := NewServer(map[Method]Handler{
		MethodRouterStart: func(context.Context, json.RawMessage) (any, *Error) {
			return nil, &Error{Code: CodePortOccupied, Message: "local port is occupied"}
		},
	})
	var output bytes.Buffer
	if err := server.Serve(context.Background(), strings.NewReader("{\"id\":\"x\",\"method\":\"router.start\",\"params\":{\"owner\":\"desktop\"}}\n"), &output); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(output.String(), `"code":"PORT_OCCUPIED"`) {
		t.Fatalf("output = %q", output.String())
	}
}

func TestServeAppliesMethodDeadline(t *testing.T) {
	server := NewServer(map[Method]Handler{
		MethodRouterStatus: func(ctx context.Context, _ json.RawMessage) (any, *Error) {
			<-ctx.Done()
			return nil, nil
		},
	})
	server.Deadlines[MethodRouterStatus] = 10 * time.Millisecond
	var output bytes.Buffer
	if err := server.Serve(context.Background(), strings.NewReader("{\"id\":\"x\",\"method\":\"router.status\"}\n"), &output); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(output.String(), `"code":"OPERATION_TIMEOUT"`) {
		t.Fatalf("output = %q", output.String())
	}
}

func TestDecodeParamsRejectsUnknownFields(t *testing.T) {
	var params RouterStartParams
	err := DecodeParams(json.RawMessage(`{"owner":"desktop","extra":true}`), &params)
	if err == nil || err.Code != CodeInvalidParams {
		t.Fatalf("error = %#v", err)
	}
}
