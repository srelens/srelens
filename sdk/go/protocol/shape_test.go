package protocol

import (
	"errors"
	"strings"
	"testing"
	"unicode"
)

func TestTheShapesAreTheBrokers(t *testing.T) {
	checks := []struct {
		name string
		f    func(string) bool
		yes  []string
		no   []string
	}{
		{"IsIdentifier", IsIdentifier, []string{"applications", strings.Repeat("a", 64)}, []string{"", strings.Repeat("a", 65), "a_b"}},
		{"IsObjectName", IsObjectName, []string{"web.v1-2", "..."}, []string{".", "..", "web/1"}},
		{"IsToken", IsToken, []string{"!", strings.Repeat("~", 128)}, []string{"u 1", "u\x7f", strings.Repeat("x", 129), "u\n1"}},
		{"IsNamespace", IsNamespace, []string{"team-1", strings.Repeat("a", 63)}, []string{"Team", "-a", "a-", ""}},
		{"IsClusterID", IsClusterID, []string{"  prod", strings.Repeat("x", 4096), "\xef\xbb\xbf"}, []string{"   ", "", strings.Repeat("x", 4097), "\xc2\xa0", "\xe3\x80\x80"}},
	}
	for _, c := range checks {
		for _, v := range c.yes {
			if !c.f(v) {
				t.Errorf("%s(%q) = false", c.name, v)
			}
		}
		for _, v := range c.no {
			if c.f(v) {
				t.Errorf("%s(%q) = true", c.name, v)
			}
		}
	}
}

// The schema's clusterId pattern, compiled by Go's regexp (RE2), tells blank
// from not blank exactly as srelens does: srelens's Rust str::trim and Go's
// strings.TrimSpace both trim Unicode White_Space.
func TestTheSchemasClusterIDPatternAgreesWithSrelens(t *testing.T) {
	notBlank := fieldShapes["CallContext.clusterId"].pattern
	var cases []string
	for _, table := range [][]unicode.Range16{unicode.White_Space.R16} {
		for _, r := range table {
			for c := rune(r.Lo); c <= rune(r.Hi); c += rune(r.Stride) {
				cases = append(cases, string(c), string(c)+"a", "a"+string(c))
			}
		}
	}
	for _, r := range unicode.White_Space.R32 {
		for c := rune(r.Lo); c <= rune(r.Hi); c += rune(r.Stride) {
			cases = append(cases, string(c))
		}
	}
	cases = append(cases, "\xef\xbb\xbf", "prod", "  prod", " ")
	for _, c := range cases {
		want := strings.TrimSpace(c) != ""
		if got := notBlank.MatchString(c); got != want {
			t.Errorf("%q: the schema says not blank = %v, srelens says %v", c, got, want)
		}
	}
}

func TestNoOperationMayTakeOneOfSrelenssOwnMethods(t *testing.T) {
	for _, name := range []string{"initialize", "health", "shutdown", "stream/open", "stream/anything", "$/cancelRequest", "$/anything"} {
		if !IsReserved(name) {
			t.Errorf("%q is not reserved", name)
		}
	}
	for _, name := range []string{"greet", "pods", "count"} {
		if IsReserved(name) {
			t.Errorf("%q is reserved", name)
		}
	}
}

func TestNewCallContextChecksBothFields(t *testing.T) {
	team := "team"
	cc, err := NewCallContext("kind-dev", &team)
	if err != nil || cc.ClusterID != "kind-dev" || cc.Namespace == nil || *cc.Namespace != "team" {
		t.Fatalf("%+v %v", cc, err)
	}
	if cc, err := NewCallContext("kind-dev", nil); err != nil || cc.Namespace != nil {
		t.Fatalf("no namespace: %+v %v", cc, err)
	}
	bad := "Team"
	for _, c := range []struct {
		cluster   string
		namespace *string
		field     string
	}{
		{"", nil, "clusterId"},
		{"\xc2\xa0", nil, "clusterId"},
		{strings.Repeat("x", 4097), nil, "clusterId"},
		{"kind-dev", &bad, "namespace"},
	} {
		_, err := NewCallContext(c.cluster, c.namespace)
		var ce *ContextError
		if !errors.As(err, &ce) || ce.Field != c.field || !strings.Contains(err.Error(), "`"+c.field+"`") {
			t.Errorf("NewCallContext(%q, %v): %v", c.cluster, c.namespace, err)
		}
	}
}
