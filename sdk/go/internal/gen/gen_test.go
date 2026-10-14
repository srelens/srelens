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

func TestBoundedBindingNameArraysGenerateTypedStrings(t *testing.T) {
	code, err := generate(minimal(`"Availability": {"type":"object","additionalProperties":false,"required":["bindings"],"properties":{"bindings":{"type":"array","minItems":1,"maxItems":16,"uniqueItems":true,"items":{"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9-]+$"}}}}`))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(code, []byte("Bindings []string")) {
		t.Fatalf("no typed binding names: %s", code)
	}
}

func TestBoundedJobInputsGenerateAStringMap(t *testing.T) {
	code, err := generate(minimal(`"Job": {"type":"object","additionalProperties":false,"required":["inputs"],"properties":{"inputs":{"type":"object","maxProperties":16,"propertyNames":{"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9-]{1,64}$"},"additionalProperties":{"type":"string","minLength":1,"maxLength":512,"pattern":"^[ -~]{1,512}$"}}}}`))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(code, []byte("Inputs map[string]string")) {
		t.Fatalf("no typed Job inputs: %s", code)
	}
}

// A top-level key the committed schema does not have, such as a new
// x-srelens-* table, would otherwise be ignored without a word.
func TestARootKeyTheGeneratorDoesNotKnowIsRefusedNamingIt(t *testing.T) {
	schema := bytes.Replace(minimal(""), []byte(`"x-srelens-apiVersion"`), []byte(`"x-srelens-streams": {}, "x-srelens-apiVersion"`), 1)
	_, err := generate(schema)
	if err == nil || !strings.Contains(err.Error(), `"x-srelens-streams"`) {
		t.Fatalf("expected a refusal naming \"x-srelens-streams\", got %v", err)
	}
}

// Only a property's own pattern is written to fieldShapes; one nested in its
// anyOf or items would be accepted and never checked.
func TestAPatternAnywhereButAPropertysTopLevelIsRefusedNamingItsPath(t *testing.T) {
	for _, c := range []struct{ prop, path string }{
		{`{"anyOf": [{"type": "string", "pattern": "^a$", "maxLength": 1}, {"type": "null"}]}`, "definitions.Thing.properties.n.anyOf"},
		{`{"type": "array", "items": {"type": "string", "pattern": "^a$", "maxLength": 1}}`, "definitions.Thing.properties.n.items"},
		{`{"type": "array", "items": {"anyOf": [{"type": "null"}, {"type": "string", "pattern": "^a$", "maxLength": 1}]}}`, "definitions.Thing.properties.n.items.anyOf"},
	} {
		_, err := generate(minimal(`"Thing": {"type": "object", "properties": {"n": ` + c.prop + `}}`))
		if err == nil || !strings.Contains(err.Error(), c.path+":") || !strings.Contains(err.Error(), "pattern") {
			t.Errorf("%s: expected a refusal naming the pattern and %s, got %v", c.prop, c.path, err)
		}
	}
}

func TestAdditionalPropertiesOtherThanFalseIsRefused(t *testing.T) {
	for _, ap := range []string{"true", "{}"} {
		_, err := generate(minimal(`"Thing": {"type": "object", "properties": {"n": {"type": "string"}}, "additionalProperties": ` + ap + `}`))
		if err == nil || !strings.Contains(err.Error(), "definitions.Thing") || !strings.Contains(err.Error(), "additionalProperties") {
			t.Errorf("additionalProperties: %s: expected a refusal naming definitions.Thing and additionalProperties, got %v", ap, err)
		}
	}
}

// methodSchema is a schema whose x-srelens-methods table holds exactly the
// given "health" method spec (a JSON fragment of its object body), for tests
// that exercise the method-table rules minimal's fixed table cannot reach.
func methodSchema(healthSpec string) []byte {
	return []byte(`{
		"definitions": {"HostMessage": {}, "SidecarMessage": {}, "RequestId": {}},
		"x-srelens-apiVersion": "0.1.0",
		"x-srelens-maxMessageBytes": 4194304,
		"x-srelens-errorCodes": {"parseError": -32700},
		"x-srelens-methods": {"health": {` + healthSpec + `}}
	}`)
}

func TestAMethodSpecKeyTheGeneratorDoesNotKnowIsRefusedNamingItAndItsPath(t *testing.T) {
	_, err := generate(methodSchema(`"direction": "hostToSidecar", "kind": "request", "params": true, "result": true, "since": "0.2.0"`))
	if err == nil || !strings.Contains(err.Error(), `"since"`) || !strings.Contains(err.Error(), "x-srelens-methods.health") {
		t.Fatalf("expected a refusal naming \"since\" and its path, got %v", err)
	}
}

func TestAMethodSpecWithAnUnknownKindIsRefused(t *testing.T) {
	_, err := generate(methodSchema(`"direction": "hostToSidecar", "kind": "stream", "params": true, "result": true`))
	if err == nil || !strings.Contains(err.Error(), "x-srelens-methods.health") || !strings.Contains(err.Error(), `"stream"`) {
		t.Fatalf("expected a refusal naming the unknown kind, got %v", err)
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
	schema, err := os.ReadFile("../../../../schemas/sidecar-protocol.v0.2.json")
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
	schema, err := os.ReadFile("../../../../schemas/sidecar-protocol.v0.2.json")
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
