package protocol

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"time"
)

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
	scanner.Buffer(make([]byte, 64*1024), 4*1024*1024)
	encoder := json.NewEncoder(output)
	encoder.SetEscapeHTML(false)

	for scanner.Scan() {
		response := s.handleLine(ctx, scanner.Bytes())
		if err := encoder.Encode(response); err != nil {
			return fmt.Errorf("write protocol response: %w", err)
		}
	}
	if err := scanner.Err(); err != nil {
		return fmt.Errorf("read protocol request: %w", err)
	}
	return nil
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
	return Response{ID: id, Error: protocolErr}
}

// DecodeParams strictly decodes method parameters and rejects unknown fields.
func DecodeParams(params json.RawMessage, target any) *Error {
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
