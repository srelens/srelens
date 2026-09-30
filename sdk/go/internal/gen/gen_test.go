package main

import (
	"bytes"
	"os"
	"strings"
	"testing"
)

// minimal is a schema with the definitions and tables the generator needs,
// plus defs, a JSON fragment of more definitions.
func minimal(defs string) []byte {
	if defs != "" {
		defs = ", " + defs
	}
	return []byte(`{
		"definitions": {"HostMessage": {}, "SidecarMessage": {}, "RequestId": {}` + defs + `},
		"x-srelens-apiVersion": "0.1.0",
		"x-srelens-maxMessageBytes": 4194304,
		"x-srelens-errorCodes": {"parseError": -32700},
		"x-srelens-methods": {"health": {"direction": "hostToSidecar", "kind": "request", "params": true, "result": true}}
	}`)
}

func TestGoNamesFollowGoInitialisms(t *testing.T) {
	for in, want := range map[string]string{
		"clusterId":             "ClusterID",
		"apiVersions":           "APIVersions",
		"uid":                   "UID",
		"cpus":                  "CPUs",
		"RpcError":              "RPCError",
		"UnsupportedApiVersion": "UnsupportedAPIVersion",
		"requestTimeoutMs":      "RequestTimeoutMs",
		"$/cancelRequest":       "CancelRequest",
		"host/read":             "HostRead",
		"resourceVersion":       "ResourceVersion",
	} {
		if got := goName(in); got != want {
			t.Errorf("goName(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestAKeywordTheGeneratorDoesNotKnowIsRefusedNamingItsPath(t *testing.T) {
	_, err := generate(minimal(`"Thing": {"type": "object", "properties": {"n": {"type": "integer", "format": "int32"}}}`))
	if err == nil || !strings.Contains(err.Error(), `"format"`) || !strings.Contains(err.Error(), "definitions.Thing.properties.n") {
		t.Fatalf("expected a refusal naming \"format\" and its path, got %v", err)
	}
}

func TestNullableIsAPointerWrittenAsNullAndOptionalIsAPointerLeftOut(t *testing.T) {
	src, err := generate(minimal(`
		"Peer": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
		"Thing": {"type": "object", "properties": {
			"namespace": {"type": ["string", "null"]},
			"sidecar": {"anyOf": [{"$ref": "#/definitions/Peer"}, {"type": "null"}]},
			"count": {"type": "integer", "minimum": 0.0},
			"offset": {"type": "integer"},
			"data": true,
			"tags": {"type": "array", "items": {"type": "string"}}
		}, "required": ["namespace", "count"]}`))
	if err != nil {
		t.Fatal(err)
	}
	text := strings.Join(strings.Fields(string(src)), " ")
	for _, want := range []string{
		"Namespace *string `json:\"namespace\"`",
		"Sidecar *Peer `json:\"sidecar,omitempty\"`",
		"Count uint64 `json:\"count\"`",
		"Offset *int64 `json:\"offset,omitempty\"`",
		"Data json.RawMessage `json:\"data,omitempty\"`",
		"Tags []string `json:\"tags,omitempty\"`",
		"Name string `json:\"name\"`",
	} {
		if !strings.Contains(text, want) {
			t.Errorf("the generated file lacks %q:\n%s", want, src)
		}
	}
}

func TestAPatternedFieldGetsItsShape(t *testing.T) {
	src, err := generate(minimal(`"Thing": {"type": "object", "properties": {
		"name": {"type": "string", "minLength": 1, "maxLength": 253, "pattern": "^[a-z.]{1,253}$", "not": {"enum": [".", ".."]}}
	}, "required": ["name"]}`))
	if err != nil {
		t.Fatal(err)
	}
	want := "\"Thing.name\": {pattern: regexp.MustCompile(\"^[a-z.]{1,253}$\"), maxLength: 253, not: []string{\".\", \"..\"}}"
	if !strings.Contains(string(src), want) {
		t.Fatalf("the generated file lacks %s:\n%s", want, src)
	}
}

func TestGenerationIsDeterministic(t *testing.T) {
	schema, err := os.ReadFile("../../../../schemas/sidecar-protocol.v0.1.json")
	if err != nil {
		t.Fatal(err)
	}
	first, err := generate(schema)
	if err != nil {
		t.Fatal(err)
	}
	for range 5 {
		again, err := generate(schema)
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(first, again) {
			t.Fatal("two runs over the same schema differ")
		}
	}
}

// The committed protocol_gen.go is what the committed schema generates. CI
// also regenerates it and fails on any difference; this says so locally.
func TestTheCommittedFileIsCurrent(t *testing.T) {
	schema, err := os.ReadFile("../../../../schemas/sidecar-protocol.v0.1.json")
	if err != nil {
		t.Fatal(err)
	}
	want, err := generate(schema)
	if err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile("../../protocol/protocol_gen.go")
	if err != nil {
		t.Fatalf("%v: run go generate ./... in sdk/go", err)
	}
	// A Windows checkout may have CRLF line endings despite .gitattributes.
	got = bytes.ReplaceAll(got, []byte("\r\n"), []byte("\n"))
	if !bytes.Equal(got, want) {
		t.Fatal("protocol/protocol_gen.go is stale: run go generate ./... in sdk/go")
	}
}
