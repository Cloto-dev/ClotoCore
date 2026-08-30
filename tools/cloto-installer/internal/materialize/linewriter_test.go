package materialize

import (
	"strings"
	"testing"
)

// The writer is exec's Stderr, so a child's output arrives in arbitrary
// chunks; lines must come out whole, once, in order, including a final
// line without a newline.
func TestLineWriterReassemblesLinesAcrossChunks(t *testing.T) {
	var got []string
	w := &lineWriter{line: func(l string) { got = append(got, l) }}
	for _, chunk := range []string{"Resol", "ved 3 packages\r\nPrep", "ared 3\n\nInstalled", " 3 packages"} {
		if _, err := w.Write([]byte(chunk)); err != nil {
			t.Fatal(err)
		}
	}
	if strings.Join(got, "|") != "Resolved 3 packages|Prepared 3|" {
		t.Errorf("before flush: %q", got)
	}
	w.flush()
	if strings.Join(got, "|") != "Resolved 3 packages|Prepared 3||Installed 3 packages" {
		t.Errorf("after flush: %q", got)
	}
	w.flush()
	if len(got) != 4 {
		t.Errorf("flush must be idempotent: %q", got)
	}
}
