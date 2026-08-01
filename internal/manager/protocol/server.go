package protocol

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"regexp"
	"strings"
	"time"
)

const maxRequestSize = 4 * 1024 * 1024

var errRequestTooLarge = errors.New("protocol request exceeds limit")

// Handler performs one manager operation. Implementations must honor context
// cancellation and complete any operation-specific cleanup before returning.
type Handler func(context.Context, json.RawMessage) (any, *Error)

// Server processes protocol requests sequentially.
type Server struct {
	Handlers  map[Method]Handler
	Deadlines map[Method]time.Duration
}

// NewServer returns a server with the required production deadlines.
func NewServer(handlers map[Method]Handler) *Server {
	return &Server{Handlers: handlers, Deadlines: Deadlines()}
}

// Serve reads one request per line and writes exactly one response per line.
// It returns cleanly at stdin EOF. Diagnostics must be written by the caller to
// stderr; this method reserves output exclusively for protocol responses.
func (s *Server) Serve(ctx context.Context, input io.Reader, output io.Writer) error {
	scanner := bufio.NewScanner(input)
	// Agent configurations can be large. Bound individual requests while
	// allowing substantially more than Scanner's 64 KiB default.
	scanner.Buffer(make([]byte, 64*1024), maxRequestSize+1)
	scanner.Split(splitBoundedLines)
	encoder := json.NewEncoder(output)
	encoder.SetEscapeHTML(false)

	for scanner.Scan() {
		response := s.handleLine(ctx, scanner.Bytes())
		if err := encoder.Encode(response); err != nil {
			return fmt.Errorf("write protocol response: %w", err)
		}
	}
	if err := scanner.Err(); err != nil {
		if errors.Is(err, errRequestTooLarge) {
			if encodeErr := encoder.Encode(errorResponse(nil, &Error{Code: CodeInvalidRequest, Message: "request exceeds size limit"})); encodeErr != nil {
				return fmt.Errorf("write protocol response: %w", encodeErr)
			}
			return nil
		}
		return fmt.Errorf("read protocol request: %w", err)
	}
	return nil
}

func splitBoundedLines(data []byte, atEOF bool) (advance int, token []byte, err error) {
	if i := bytes.IndexByte(data, '\n'); i >= 0 {
		if i > maxRequestSize {
			return 0, nil, errRequestTooLarge
		}
		return i + 1, dropCarriageReturn(data[:i]), nil
	}
	if len(data) > maxRequestSize {
		return 0, nil, errRequestTooLarge
	}
	if atEOF && len(data) != 0 {
		return len(data), dropCarriageReturn(data), nil
	}
	return 0, nil, nil
}

func dropCarriageReturn(data []byte) []byte {
	if len(data) != 0 && data[len(data)-1] == '\r' {
		return data[:len(data)-1]
	}
	return data
}

func (s *Server) handleLine(parent context.Context, line []byte) Response {
	request, protocolErr := decodeRequest(line)
	if protocolErr != nil {
		return errorResponse(nil, protocolErr)
	}

	id := request.ID
	handler, ok := s.Handlers[request.Method]
	if !ok {
		if _, known := s.Deadlines[request.Method]; known {
			return errorResponse(&id, &Error{Code: CodeInvalidRequest, Message: "method is unavailable"})
		}
		return errorResponse(&id, &Error{Code: CodeUnknownMethod, Message: "unknown method"})
	}
	deadline, ok := s.Deadlines[request.Method]
	if !ok || deadline <= 0 {
		return errorResponse(&id, &Error{Code: CodeInvalidRequest, Message: "method deadline is unavailable"})
	}

	ctx, cancel := context.WithTimeout(parent, deadline)
	defer cancel()
	result, operationErr := handler(ctx, normalizeParams(request.Params))
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return errorResponse(&id, &Error{Code: CodeOperationTimeout, Message: "operation timed out"})
	}
	if operationErr != nil {
		return errorResponse(&id, operationErr)
	}
	encoded, err := json.Marshal(result)
	if err != nil {
		return errorResponse(&id, &Error{Code: CodeInvalidRequest, Message: "operation returned an invalid result"})
	}
	return Response{ID: &id, Result: encoded}
}

func decodeRequest(line []byte) (Request, *Error) {
	decoder := json.NewDecoder(bytes.NewReader(line))
	decoder.DisallowUnknownFields()
	var request Request
	if err := decoder.Decode(&request); err != nil {
		return Request{}, &Error{Code: CodeInvalidRequest, Message: "malformed request"}
	}
	if decoder.Decode(&struct{}{}) != io.EOF {
		return Request{}, &Error{Code: CodeInvalidRequest, Message: "malformed request"}
	}
	if request.ID == "" || request.Method == "" {
		return Request{}, &Error{Code: CodeInvalidRequest, Message: "id and method are required"}
	}
	if len(request.Params) != 0 && !json.Valid(request.Params) {
		return Request{}, &Error{Code: CodeInvalidRequest, Message: "params must be valid JSON"}
	}
	return request, nil
}

func normalizeParams(params json.RawMessage) json.RawMessage {
	if len(params) == 0 || bytes.Equal(bytes.TrimSpace(params), []byte("null")) {
		return json.RawMessage(`{}`)
	}
	return params
}

func errorResponse(id *string, protocolErr *Error) Response {
	boundErrorDetails(protocolErr)
	return Response{ID: id, Error: protocolErr}
}

var validationRulePattern = regexp.MustCompile(`^[a-z][a-z0-9_]{0,63}$`)

func boundErrorDetails(protocolErr *Error) {
	if protocolErr == nil || protocolErr.Details == nil {
		return
	}
	if protocolErr.Code != CodeInvalidParams && protocolErr.Code != CodeModelConfigInvalid {
		protocolErr.Details = nil
		return
	}
	details := protocolErr.Details
	if len(details.Path) > 1024 || (details.Path != "" && !strings.HasPrefix(details.Path, "/")) ||
		!validationRulePattern.MatchString(details.Rule) {
		protocolErr.Details = nil
	}
}

// DecodeParams strictly decodes method parameters and rejects duplicate or
// unknown fields at any object depth.
func DecodeParams(params json.RawMessage, target any) *Error {
	if err := rejectDuplicateJSONKeys(params); err != nil {
		return &Error{Code: CodeInvalidParams, Message: "invalid method parameters"}
	}
	decoder := json.NewDecoder(bytes.NewReader(params))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		return &Error{Code: CodeInvalidParams, Message: "invalid method parameters"}
	}
	if decoder.Decode(&struct{}{}) != io.EOF {
		return &Error{Code: CodeInvalidParams, Message: "invalid method parameters"}
	}
	return nil
}

func rejectDuplicateJSONKeys(data []byte) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if err := scanUniqueJSONValue(decoder); err != nil {
		return err
	}
	if _, err := decoder.Token(); err != io.EOF {
		if err == nil {
			return errors.New("trailing JSON value")
		}
		return err
	}
	return nil
}

func scanUniqueJSONValue(decoder *json.Decoder) error {
	token, err := decoder.Token()
	if err != nil {
		return err
	}
	delimiter, ok := token.(json.Delim)
	if !ok {
		return nil
	}
	switch delimiter {
	case '{':
		keys := make(map[string]struct{})
		for decoder.More() {
			token, err := decoder.Token()
			if err != nil {
				return err
			}
			key, ok := token.(string)
			if !ok {
				return errors.New("invalid JSON object key")
			}
			if _, exists := keys[key]; exists {
				return errors.New("duplicate JSON object key")
			}
			keys[key] = struct{}{}
			if err := scanUniqueJSONValue(decoder); err != nil {
				return err
			}
		}
		closing, err := decoder.Token()
		if err != nil || closing != json.Delim('}') {
			return errors.New("invalid JSON object")
		}
	case '[':
		for decoder.More() {
			if err := scanUniqueJSONValue(decoder); err != nil {
				return err
			}
		}
		closing, err := decoder.Token()
		if err != nil || closing != json.Delim(']') {
			return errors.New("invalid JSON array")
		}
	default:
		return errors.New("invalid JSON delimiter")
	}
	return nil
}
