package main

import (
	"flag"
	"fmt"
	"os"
)

// Run by `go generate` in sdk/go/protocol, so the defaults are relative to it.
func main() {
	schemaPath := flag.String("schema", "../../../schemas/sidecar-protocol.v0.2.json", "the committed protocol schema")
	outPath := flag.String("out", "protocol_gen.go", "the file to write")
	flag.Parse()
	schema, err := os.ReadFile(*schemaPath)
	if err != nil {
		fail(err)
	}
	src, err := generate(schema)
	if err != nil {
		fail(err)
	}
	if err := os.WriteFile(*outPath, src, 0o644); err != nil {
		fail(err)
	}
}

func fail(err error) {
	fmt.Fprintln(os.Stderr, "gen:", err)
	os.Exit(1)
}
