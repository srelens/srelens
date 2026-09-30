package protocol

import (
	"encoding/json"
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
