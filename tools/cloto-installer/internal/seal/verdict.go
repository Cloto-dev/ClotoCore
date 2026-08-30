package seal

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"

	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/catalog"
)

// Domain-separation tag mixed into every Ed25519 signing input.
const ed25519Domain = "mgp-seal-ed25519-v1"

// keyIDMaxLen bounds a JWKS `kid` in UTF-8 bytes.
const keyIDMaxLen = 256

// Reasons an install proceeds without a local seal. The names are the
// wire vocabulary shared with the kernel and the fixtures.
const (
	ReasonNoEntryPointHash        = "no_entry_point_hash"
	ReasonNoSignature             = "no_signature"
	ReasonMalformedSignatureBlock = "malformed_signature_block"
	ReasonUndecodableSignature    = "undecodable_signature"
	ReasonHubKeyUnavailable       = "hub_key_unavailable"
	ReasonMalformedArchiveBinding = "malformed_archive_binding"
)

// Tamper codes: a well-formed, resolvable claim that does not match what
// was delivered.
const (
	CodeIntegrityMismatch = "integrity_mismatch"
	CodeSignatureInvalid  = "signature_invalid"
)

// Verdict is the install-time decision on one catalog entry.
type Verdict struct {
	// `verified`, `unsealed` or `tamper`.
	Kind string `json:"verdict"`
	// Set for `unsealed`.
	Reason string `json:"reason,omitempty"`
	// Set for `tamper`.
	Code string `json:"code,omitempty"`
	// Human-readable explanation for `tamper`.
	Message string `json:"message,omitempty"`
}

// ArchiveBinding is what a `dual-v2` seal says about the archive it was
// signed over.
type ArchiveBinding struct {
	// `absent` (v1 seal or none), `malformed` (an archive block that
	// cannot be used) or `bound`.
	State  string
	SHA256 string
	Length uint64
}

// ReadArchiveBinding reads the archive binding a `dual-v2` seal carries.
// The values are claims until the signature over them verifies.
func ReadArchiveBinding(entry *catalog.Entry) ArchiveBinding {
	archive := entry.SignatureField("archive")
	if archive == nil {
		return ArchiveBinding{State: "absent"}
	}
	var sha string
	if raw, ok := archive["sha256"]; !ok || json.Unmarshal(raw, &sha) != nil {
		return ArchiveBinding{State: "malformed"}
	}
	rawLen, ok := archive["length"]
	if !ok {
		return ArchiveBinding{State: "malformed"}
	}
	length, ok := asUint64(rawLen)
	if !ok {
		return ArchiveBinding{State: "malformed"}
	}
	sha = strings.TrimSpace(sha)
	if !isHex(sha, 64) {
		return ArchiveBinding{State: "malformed"}
	}
	return ArchiveBinding{State: "bound", SHA256: strings.ToLower(sha), Length: length}
}

// asUint64 accepts a JSON number that is a non-negative integer, the way
// the kernel's `as_u64` does; a float or a string is not a length.
func asUint64(raw json.RawMessage) (uint64, bool) {
	s := strings.TrimSpace(string(raw))
	if s == "" || strings.ContainsAny(s, ".eE\"") {
		return 0, false
	}
	v, err := strconv.ParseUint(s, 10, 64)
	if err != nil {
		return 0, false
	}
	return v, true
}

func isHex(s string, length int) bool {
	if len(s) != length {
		return false
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		if !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F')) {
			return false
		}
	}
	return true
}

// CanonicalMessageV1 is the signed message for a seal that binds only the
// entry point.
func CanonicalMessageV1(connectorID, version, entryPointSHA256 string) []byte {
	return []byte("mgp-seal/v1\nconnector_id=" + connectorID + "\nversion=" + version +
		"\nentry_point_sha256=" + entryPointSHA256 + "\n")
}

// CanonicalMessageV2 is the signed message for a seal that also binds the
// distributed archive's digest and length.
func CanonicalMessageV2(connectorID, version, entryPointSHA256, archiveSHA256 string, archiveLength uint64) []byte {
	return []byte("mgp-seal/v2\nconnector_id=" + connectorID + "\nversion=" + version +
		"\nentry_point_sha256=" + entryPointSHA256 + "\narchive_sha256=" + archiveSHA256 +
		"\narchive_length=" + strconv.FormatUint(archiveLength, 10) + "\n")
}

// VerifyEd25519 checks sig over message under key, with keyID mixed into
// the signing input: `domain || 0x00 || key_id || 0x00 || message`.
func VerifyEd25519(key ed25519.PublicKey, keyID string, message, sig []byte) bool {
	input := make([]byte, 0, len(ed25519Domain)+1+len(keyID)+1+len(message))
	input = append(input, ed25519Domain...)
	input = append(input, 0)
	input = append(input, keyID...)
	input = append(input, 0)
	input = append(input, message...)
	return ed25519.Verify(key, input, sig)
}

// ParseJWK reads one RFC 8037 OKP/Ed25519 JWK. Validation is strict:
// `kty` OKP, `crv` Ed25519, `alg` EdDSA and `use` sig when present, `kid`
// non-empty and bounded, `x` base64url without padding decoding to 32
// bytes.
func ParseJWK(jwk map[string]json.RawMessage) (ed25519.PublicKey, string, error) {
	field := func(name string) (string, bool) {
		raw, ok := jwk[name]
		if !ok {
			return "", false
		}
		var s string
		if err := json.Unmarshal(raw, &s); err != nil {
			return "", false
		}
		return s, true
	}
	if v, ok := field("kty"); !ok || v != "OKP" {
		return nil, "", fmt.Errorf("JWK `kty` must be \"OKP\"")
	}
	if v, ok := field("crv"); !ok || v != "Ed25519" {
		return nil, "", fmt.Errorf("JWK `crv` must be \"Ed25519\"")
	}
	if v, ok := field("alg"); ok && v != "EdDSA" {
		return nil, "", fmt.Errorf("JWK `alg` must be \"EdDSA\" when present")
	}
	if v, ok := field("use"); ok && v != "sig" {
		return nil, "", fmt.Errorf("JWK `use` must be \"sig\" when present")
	}
	kid, ok := field("kid")
	if !ok {
		return nil, "", fmt.Errorf("JWK is missing the `kid` field")
	}
	if kid == "" || len(kid) > keyIDMaxLen {
		return nil, "", fmt.Errorf("JWK `kid` must be non-empty and at most %d bytes", keyIDMaxLen)
	}
	x, ok := field("x")
	if !ok {
		return nil, "", fmt.Errorf("JWK is missing the `x` field")
	}
	raw, err := base64.RawURLEncoding.Strict().DecodeString(x)
	if err != nil {
		return nil, "", fmt.Errorf("JWK `x` is not valid base64url-no-pad")
	}
	if len(raw) != ed25519.PublicKeySize {
		return nil, "", fmt.Errorf("JWK `x` must decode to exactly %d bytes (got %d)", ed25519.PublicKeySize, len(raw))
	}
	return ed25519.PublicKey(raw), kid, nil
}

// KeyFromJWKS resolves kid in a JWKS document (`{"keys":[...]}`), the way
// the kernel does: unparseable keys are skipped, an unknown kid yields nil.
// A nil or empty document (the hub was unreachable) also yields nil.
func KeyFromJWKS(jwks json.RawMessage, kid string) ed25519.PublicKey {
	if len(jwks) == 0 || string(jwks) == "null" {
		return nil
	}
	var doc struct {
		Keys []map[string]json.RawMessage `json:"keys"`
	}
	if err := json.Unmarshal(jwks, &doc); err != nil {
		return nil
	}
	for _, jwk := range doc.Keys {
		key, id, err := ParseJWK(jwk)
		if err != nil {
			continue
		}
		if id == kid {
			return key
		}
	}
	return nil
}

// SignatureKeyID returns the `key_id` named by the entry's ed25519 block,
// or "" when there is none.
func SignatureKeyID(entry *catalog.Entry) string {
	block := entry.SignatureField("ed25519")
	if block == nil {
		return ""
	}
	var kid string
	if raw, ok := block["key_id"]; ok && json.Unmarshal(raw, &kid) == nil {
		return kid
	}
	return ""
}

// Decide computes the install-time verdict for entry.
//
// installedEntryPointSHA256 is only called once the catalog has recorded a
// hash to compare against; an entry without one never has its entry point
// read. An error from it aborts the decision (a file that cannot be hashed
// cannot be verified). hubKey is the signing key pre-resolved for the
// entry's `key_id`; nil means the hub key is unavailable.
//
// The order matters and is the kernel's: the keyless integrity check
// first (a hash mismatch is the strongest tamper signal regardless of
// signature state), then the Ed25519 layer.
func Decide(entry *catalog.Entry, installedEntryPointSHA256 func() (string, error), hubKey ed25519.PublicKey) (Verdict, error) {
	ed := entry.SignatureField("ed25519")

	expected := ""
	if entry.EntryPointSHA256 != nil {
		expected = strings.TrimSpace(*entry.EntryPointSHA256)
	}
	if expected == "" {
		return Verdict{Kind: "unsealed", Reason: ReasonNoEntryPointHash}, nil
	}

	actual, err := installedEntryPointSHA256()
	if err != nil {
		return Verdict{}, err
	}
	if !strings.EqualFold(actual, expected) {
		return Verdict{
			Kind: "tamper",
			Code: CodeIntegrityMismatch,
			Message: fmt.Sprintf(
				"entry point integrity check failed for '%s': catalog expects sha256 %s, installed file hashes to %s",
				entry.ID, expected, actual),
		}, nil
	}

	if ed == nil {
		return Verdict{Kind: "unsealed", Reason: ReasonNoSignature}, nil
	}
	var sigB64, kid string
	rawSig, okSig := ed["sig"]
	rawKid, okKid := ed["key_id"]
	if !okSig || !okKid || json.Unmarshal(rawSig, &sigB64) != nil || json.Unmarshal(rawKid, &kid) != nil {
		return Verdict{Kind: "unsealed", Reason: ReasonMalformedSignatureBlock}, nil
	}
	sig, err := base64.StdEncoding.Strict().DecodeString(strings.TrimSpace(sigB64))
	if err != nil || len(sig) != ed25519.SignatureSize || kid == "" || len(kid) > keyIDMaxLen {
		return Verdict{Kind: "unsealed", Reason: ReasonUndecodableSignature}, nil
	}
	if hubKey == nil {
		return Verdict{Kind: "unsealed", Reason: ReasonHubKeyUnavailable}, nil
	}

	// A `dual-v2` seal signed the archive alongside the entry point, so
	// the message to reconstruct includes it. The two versions are
	// domain-separated by their first line; a binding that cannot be
	// parsed must not fall back to v1, which would report a genuine
	// hub-issued seal as tampered.
	var canonical []byte
	switch binding := ReadArchiveBinding(entry); binding.State {
	case "bound":
		canonical = CanonicalMessageV2(entry.ID, entry.Version, expected, binding.SHA256, binding.Length)
	case "absent":
		canonical = CanonicalMessageV1(entry.ID, entry.Version, expected)
	default:
		return Verdict{Kind: "unsealed", Reason: ReasonMalformedArchiveBinding}, nil
	}
	if !VerifyEd25519(hubKey, kid, canonical, sig) {
		return Verdict{
			Kind: "tamper",
			Code: CodeSignatureInvalid,
			Message: fmt.Sprintf(
				"Ed25519 seal verification failed for '%s': the catalog's signature does not match its signed identity under hub key '%s' — refusing install",
				entry.ID, kid),
		}, nil
	}
	return Verdict{Kind: "verified"}, nil
}
