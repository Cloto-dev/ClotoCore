package seal

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/catalog"
)

type catalogCases struct {
	JWKS  string `json:"jwks"`
	Cases []struct {
		Name                      string        `json:"name"`
		Entry                     catalog.Entry `json:"entry"`
		InstalledEntryPointSHA256 string        `json:"installed_entry_point_sha256"`
		HubKey                    *string       `json:"hub_key"`
		Expect                    struct {
			Verdict string  `json:"verdict"`
			Reason  *string `json:"reason"`
			Code    *string `json:"code"`
		} `json:"expect"`
	} `json:"cases"`
}

func TestInstallSealVerdictAgreesWithEveryRecordedCase(t *testing.T) {
	root := filepath.Join(fixtures(t), "catalog")
	var cases catalogCases
	readJSON(t, filepath.Join(root, "cases.json"), &cases)
	jwks, err := os.ReadFile(filepath.Join(root, cases.JWKS))
	if err != nil {
		t.Fatal(err)
	}
	verdictsSeen := map[string]bool{}
	verifiedV1, verifiedV2 := false, false

	for _, c := range cases.Cases {
		c.Entry.Normalize()
		var hubKey ed25519.PublicKey
		if c.HubKey != nil {
			hubKey = KeyFromJWKS(jwks, *c.HubKey)
			if hubKey == nil {
				t.Fatalf("%s: kid %q not in fixture JWKS", c.Name, *c.HubKey)
			}
		}
		read := false
		installed := func() (string, error) {
			read = true
			return c.InstalledEntryPointSHA256, nil
		}
		got, err := Decide(&c.Entry, installed, hubKey)
		if err != nil {
			t.Fatalf("%s: %v", c.Name, err)
		}
		switch c.Expect.Verdict {
		case "verified":
			if got.Kind != "verified" {
				t.Errorf("%s: want verified, got %+v", c.Name, got)
			}
			if c.Entry.SignatureField("archive") != nil {
				verifiedV2 = true
			} else {
				verifiedV1 = true
			}
		case "unsealed":
			if c.Expect.Reason == nil {
				t.Fatalf("%s: unsealed case names no reason", c.Name)
			}
			if got.Kind != "unsealed" || got.Reason != *c.Expect.Reason {
				t.Errorf("%s: want unsealed/%s, got %+v", c.Name, *c.Expect.Reason, got)
			}
		case "tamper":
			if c.Expect.Code == nil {
				t.Fatalf("%s: tamper case names no code", c.Name)
			}
			if got.Kind != "tamper" || got.Code != *c.Expect.Code {
				t.Errorf("%s: want tamper/%s, got %+v", c.Name, *c.Expect.Code, got)
			}
		default:
			t.Fatalf("unknown verdict %q in %q", c.Expect.Verdict, c.Name)
		}
		// An entry without a catalog hash must be decided without touching
		// the installed file.
		if c.InstalledEntryPointSHA256 == "never-read" && read {
			t.Errorf("%s: entry point was read", c.Name)
		}
		verdictsSeen[c.Expect.Verdict] = true
	}
	for _, want := range []string{"tamper", "unsealed", "verified"} {
		if !verdictsSeen[want] {
			t.Errorf("fixture exercises no %q case", want)
		}
	}
	if !verifiedV1 {
		t.Error("fixture must carry a verified v1 (entry point only) entry")
	}
	if !verifiedV2 {
		t.Error("fixture must carry a verified v2 (archive-bound) entry")
	}
}

// The recorded live entries are the point of the fixture: real hub
// signatures under the real hub key.
func TestCatalogFixtureUsesTheHubSigningKey(t *testing.T) {
	root := filepath.Join(fixtures(t), "catalog")
	var cases catalogCases
	readJSON(t, filepath.Join(root, "cases.json"), &cases)
	jwks, err := os.ReadFile(filepath.Join(root, cases.JWKS))
	if err != nil {
		t.Fatal(err)
	}
	named := map[string]bool{}
	for _, c := range cases.Cases {
		if c.HubKey != nil {
			named[*c.HubKey] = true
		}
	}
	if len(named) == 0 {
		t.Fatal("no case names a hub key")
	}
	for kid := range named {
		if KeyFromJWKS(jwks, kid) == nil {
			t.Errorf("kid %q not in fixture JWKS", kid)
		}
	}
}

func TestCanonicalMessagesPinExactBytes(t *testing.T) {
	v1 := string(CanonicalMessageV1("cpersona", "1.2.3", "deadbeef"))
	if v1 != "mgp-seal/v1\nconnector_id=cpersona\nversion=1.2.3\nentry_point_sha256=deadbeef\n" {
		t.Errorf("v1: %q", v1)
	}
	v2 := string(CanonicalMessageV2("cpersona", "1.2.3", "deadbeef", "cafe", 42))
	if v2 != "mgp-seal/v2\nconnector_id=cpersona\nversion=1.2.3\nentry_point_sha256=deadbeef\narchive_sha256=cafe\narchive_length=42\n" {
		t.Errorf("v2: %q", v2)
	}
}

// The key id is part of the signed input: a signature under one kid must
// not verify under another, and the domain tag must be present.
func TestEd25519DomainSeparationBindsTheKeyID(t *testing.T) {
	pub, priv, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	msg := []byte("payload")
	input := append(append(append(append([]byte(ed25519Domain), 0), "kid-a"...), 0), msg...)
	sig := ed25519.Sign(priv, input)
	if !VerifyEd25519(pub, "kid-a", msg, sig) {
		t.Error("signature under kid-a must verify under kid-a")
	}
	if VerifyEd25519(pub, "kid-b", msg, sig) {
		t.Error("signature under kid-a must not verify under kid-b")
	}
	if VerifyEd25519(pub, "kid-a", msg, ed25519.Sign(priv, msg)) {
		t.Error("a signature over the bare message lacks the domain tag and must fail")
	}
}

func TestJWKParsingIsStrict(t *testing.T) {
	pub, _, _ := ed25519.GenerateKey(rand.Reader)
	x := base64.RawURLEncoding.EncodeToString(pub)
	good := map[string]json.RawMessage{
		"kty": json.RawMessage(`"OKP"`), "crv": json.RawMessage(`"Ed25519"`),
		"alg": json.RawMessage(`"EdDSA"`), "use": json.RawMessage(`"sig"`),
		"kid": json.RawMessage(`"k1"`), "x": json.RawMessage(`"` + x + `"`),
	}
	if _, kid, err := ParseJWK(good); err != nil || kid != "k1" {
		t.Fatalf("good JWK rejected: %v", err)
	}
	mutate := func(field, value string) map[string]json.RawMessage {
		m := map[string]json.RawMessage{}
		for k, v := range good {
			m[k] = v
		}
		if value == "" {
			delete(m, field)
		} else {
			m[field] = json.RawMessage(value)
		}
		return m
	}
	for name, jwk := range map[string]map[string]json.RawMessage{
		"wrong kty":     mutate("kty", `"RSA"`),
		"wrong crv":     mutate("crv", `"P-256"`),
		"wrong alg":     mutate("alg", `"ES256"`),
		"wrong use":     mutate("use", `"enc"`),
		"missing kid":   mutate("kid", ""),
		"empty kid":     mutate("kid", `""`),
		"missing x":     mutate("x", ""),
		"padded x":      mutate("x", `"`+base64.URLEncoding.EncodeToString(pub)+`"`),
		"short x":       mutate("x", `"`+base64.RawURLEncoding.EncodeToString(pub[:16])+`"`),
		"standard b64x": mutate("x", `"`+base64.RawStdEncoding.EncodeToString([]byte{0xfb, 0xff, 0xfe, 0xfb, 0xff, 0xfe, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26})+`"`),
	} {
		if _, _, err := ParseJWK(jwk); err == nil {
			t.Errorf("%s: accepted", name)
		}
	}
	// A JWK without alg / use is still fine.
	if _, _, err := ParseJWK(mutate("alg", "")); err != nil {
		t.Errorf("alg is optional: %v", err)
	}
}

func TestArchiveBindingRejectsNonIntegerLength(t *testing.T) {
	entry := func(payload string) *catalog.Entry {
		return &catalog.Entry{SignaturePayload: json.RawMessage(payload)}
	}
	sha := `"` + "ab" + string(make([]byte, 0)) + `"`
	_ = sha
	hex64 := `"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"`
	for name, payload := range map[string]string{
		"float length":  `{"archive":{"sha256":` + hex64 + `,"length":266814.0}}`,
		"string length": `{"archive":{"sha256":` + hex64 + `,"length":"266814"}}`,
		"negative":      `{"archive":{"sha256":` + hex64 + `,"length":-1}}`,
		"short digest":  `{"archive":{"sha256":"abcd","length":1}}`,
	} {
		if b := ReadArchiveBinding(entry(payload)); b.State != "malformed" {
			t.Errorf("%s: got %+v, want malformed", name, b)
		}
	}
	if b := ReadArchiveBinding(entry(`{"archive":{"sha256":` + hex64 + `,"length":266814}}`)); b.State != "bound" || b.Length != 266814 {
		t.Errorf("integer length: got %+v", b)
	}
	if b := ReadArchiveBinding(entry(`{"ed25519":{}}`)); b.State != "absent" {
		t.Errorf("no archive block: got %+v", b)
	}
}
