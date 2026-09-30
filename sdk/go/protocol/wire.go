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
