package protocol

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
)

// RequestID is a JSON-RPC request id: a number or a string. 1 and "1" are
// different ids.
type RequestID struct {
	text string // its JSON text
}

// NumberID is the id n.
func NumberID(n int64) RequestID { return RequestID{text: strconv.FormatInt(n, 10)} }

// StringID is the id s.
func StringID(s string) RequestID {
	b, _ := json.Marshal(s)
	return RequestID{text: string(b)}
}

// Key is the id's JSON text: one per id, and different for 1 and "1".
func (id RequestID) Key() string { return id.text }

// IsZero reports whether the id was never set.
func (id RequestID) IsZero() bool { return id.text == "" }

func (id RequestID) String() string { return id.text }

func (id RequestID) MarshalJSON() ([]byte, error) {
	if id.text == "" {
		return nil, errors.New("a request id was never set")
	}
	return []byte(id.text), nil
}

func (id *RequestID) UnmarshalJSON(b []byte) error {
	dec := json.NewDecoder(bytes.NewReader(b))
	dec.UseNumber()
	var v any
	if err := dec.Decode(&v); err != nil {
		return err
	}
	switch v := v.(type) {
	case json.Number:
		*id = RequestID{text: v.String()}
	case string:
		*id = StringID(v)
	default:
		return fmt.Errorf("an id is a number or a string, not %s", b)
	}
	return nil
}

// Request is a JSON-RPC request.
type Request struct {
	ID     RequestID
	Method string
	Params json.RawMessage
}

func (r Request) MarshalJSON() ([]byte, error) {
	return json.Marshal(struct {
		JSONRPC string          `json:"jsonrpc"`
		ID      RequestID       `json:"id"`
		Method  string          `json:"method"`
		Params  json.RawMessage `json:"params,omitempty"`
	}{"2.0", r.ID, r.Method, r.Params})
}

// Notification is a request with no id, which is never answered.
type Notification struct {
	Method string
	Params json.RawMessage
}

func (n Notification) MarshalJSON() ([]byte, error) {
	return json.Marshal(struct {
		JSONRPC string          `json:"jsonrpc"`
		Method  string          `json:"method"`
		Params  json.RawMessage `json:"params,omitempty"`
	}{"2.0", n.Method, n.Params})
}

// Response answers request ID: with Error when it is set, and with Result
// otherwise (null when Result is empty). It is written with exactly one of
// the two.
type Response struct {
	ID     RequestID
	Result json.RawMessage
	Error  *RPCError
}

func (r Response) MarshalJSON() ([]byte, error) {
	if r.Error != nil {
		return json.Marshal(struct {
			JSONRPC string    `json:"jsonrpc"`
			ID      RequestID `json:"id"`
			Error   *RPCError `json:"error"`
		}{"2.0", r.ID, r.Error})
	}
	result := r.Result
	if len(result) == 0 {
		result = json.RawMessage("null")
	}
	return json.Marshal(struct {
		JSONRPC string          `json:"jsonrpc"`
		ID      RequestID       `json:"id"`
		Result  json.RawMessage `json:"result"`
	}{"2.0", r.ID, result})
}

// Message is one parsed line: exactly one of its fields is set.
type Message struct {
	Request      *Request
	Notification *Notification
	Response     *Response
}

// ParseMessage reads one line as a JSON-RPC 2.0 message: a request (an id and
// a method), a notification (a method and no id) or a response (an id, no
// method, and exactly one of result and error).
func ParseMessage(line []byte) (Message, error) {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(line, &fields); err != nil || fields == nil {
		return Message{}, errors.New("a message is a JSON object")
	}
	if v, ok := fields["jsonrpc"]; !ok || string(bytes.TrimSpace(v)) != `"2.0"` {
		return Message{}, errors.New(`a message's "jsonrpc" must be "2.0"`)
	}
	rawID, hasID := fields["id"]
	rawMethod, hasMethod := fields["method"]
	var id RequestID
	if hasID {
		if err := json.Unmarshal(rawID, &id); err != nil {
			return Message{}, fmt.Errorf(`"id": %w`, err)
		}
	}
	if hasMethod {
		var method string
		if err := json.Unmarshal(rawMethod, &method); err != nil {
			return Message{}, errors.New(`"method" must be a string`)
		}
		if hasID {
			return Message{Request: &Request{ID: id, Method: method, Params: fields["params"]}}, nil
		}
		return Message{Notification: &Notification{Method: method, Params: fields["params"]}}, nil
	}
	if !hasID {
		return Message{}, errors.New(`a message has a "method", an "id", or both`)
	}
	rawResult, hasResult := fields["result"]
	rawError, hasError := fields["error"]
	if hasResult == hasError {
		return Message{}, errors.New(`a response has exactly one of "result" and "error"`)
	}
	if hasError {
		var e RPCError
		if err := json.Unmarshal(rawError, &e); err != nil {
			return Message{}, fmt.Errorf(`"error": %w`, err)
		}
		return Message{Response: &Response{ID: id, Error: &e}}, nil
	}
	return Message{Response: &Response{ID: id, Result: rawResult}}, nil
}
