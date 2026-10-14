package protocol

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

func TestARequestIDRoundTripsAndOneIsNotTheStringOne(t *testing.T) {
	var number, text RequestID
	if err := json.Unmarshal([]byte(`1`), &number); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal([]byte(`"1"`), &text); err != nil {
		t.Fatal(err)
	}
	if number.Key() == text.Key() {
		t.Fatalf("1 and \"1\" share the key %s", number.Key())
	}
	if number != NumberID(1) || text != StringID("1") {
		t.Fatalf("parsed %v and %v", number, text)
	}
	for _, id := range []RequestID{NumberID(7), StringID("c-1")} {
		b, err := json.Marshal(id)
		if err != nil {
			t.Fatal(err)
		}
		var back RequestID
		if err := json.Unmarshal(b, &back); err != nil || back != id {
			t.Fatalf("%s read back as %v (%v)", b, back, err)
		}
	}
}

func TestAnIDIsANumberOrAString(t *testing.T) {
	for _, bad := range []string{`null`, `true`, `{}`, `[1]`} {
		var id RequestID
		if err := json.Unmarshal([]byte(bad), &id); err == nil {
			t.Errorf("%s was read as an id", bad)
		}
	}
	if !(RequestID{}).IsZero() || NumberID(1).IsZero() {
		t.Fatal("IsZero is wrong")
	}
	if _, err := json.Marshal(RequestID{}); err == nil {
		t.Fatal("an id that was never set was written")
	}
}

func TestParseMessageTellsARequestANotificationAndAResponseApart(t *testing.T) {
	m, err := ParseMessage([]byte(`{"jsonrpc":"2.0","id":3,"method":"greet","params":{"name":"x"}}` + "\n"))
	if err != nil || m.Request == nil || m.Request.ID != NumberID(3) || m.Request.Method != "greet" || string(m.Request.Params) != `{"name":"x"}` {
		t.Fatalf("a request: %+v %v", m, err)
	}
	m, err = ParseMessage([]byte(`{"jsonrpc":"2.0","method":"stream/cancel","params":{"stream":1}}`))
	if err != nil || m.Notification == nil || m.Notification.Method != "stream/cancel" {
		t.Fatalf("a notification: %+v %v", m, err)
	}
	m, err = ParseMessage([]byte(`{"jsonrpc":"2.0","id":"c-1","result":null}`))
	if err != nil || m.Response == nil || m.Response.ID != StringID("c-1") || string(m.Response.Result) != "null" || m.Response.Error != nil {
		t.Fatalf("a null result: %+v %v", m, err)
	}
	m, err = ParseMessage([]byte(`{"jsonrpc":"2.0","id":"c-2","error":{"code":-32002,"message":"declined","data":{"why":1}}}`))
	if err != nil || m.Response == nil || m.Response.Error == nil || m.Response.Error.Code != -32002 || string(m.Response.Error.Data) != `{"why":1}` {
		t.Fatalf("an error: %+v %v", m, err)
	}
}

func TestParseMessageRefusesWhatIsNotJSONRPC(t *testing.T) {
	for _, line := range []string{
		`not json`, `null`, `[1]`, `{}`,
		`{"id":1,"method":"health"}`,
		`{"jsonrpc":"1.0","id":1,"method":"health"}`,
		`{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1,"message":"x"}}`,
		`{"jsonrpc":"2.0","id":1}`,
		`{"jsonrpc":"2.0","id":true,"method":"health"}`,
		`{"jsonrpc":"2.0","id":1,"method":7}`,
	} {
		if m, err := ParseMessage([]byte(line)); err == nil {
			t.Errorf("%s was read as %+v", line, m)
		}
	}
}

func TestAResponseWritesExactlyOneOfResultAndError(t *testing.T) {
	cases := map[string]Response{
		`{"jsonrpc":"2.0","id":1,"result":{}}`:                                {ID: NumberID(1), Result: json.RawMessage(`{}`)},
		`{"jsonrpc":"2.0","id":1,"result":null}`:                              {ID: NumberID(1)},
		`{"jsonrpc":"2.0","id":"c-1","error":{"code":-32601,"message":"no"}}`: {ID: StringID("c-1"), Error: &RPCError{Code: -32601, Message: "no"}},
	}
	for want, r := range cases {
		b, err := json.Marshal(r)
		if err != nil {
			t.Fatal(err)
		}
		var got, expected any
		_ = json.Unmarshal(b, &got)
		_ = json.Unmarshal([]byte(want), &expected)
		if !reflect.DeepEqual(got, expected) {
			t.Errorf("wrote %s, want %s", b, want)
		}
	}
	b, _ := json.Marshal(Request{ID: StringID("c-1"), Method: "host/read", Params: json.RawMessage(`{}`)})
	if !strings.Contains(string(b), `"jsonrpc":"2.0"`) {
		t.Fatalf("a request without jsonrpc: %s", b)
	}
	b, _ = json.Marshal(Notification{Method: "stream/close", Params: json.RawMessage(`{"stream":1}`)})
	if strings.Contains(string(b), `"id"`) || !strings.Contains(string(b), `"jsonrpc":"2.0"`) {
		t.Fatalf("a notification: %s", b)
	}
}
