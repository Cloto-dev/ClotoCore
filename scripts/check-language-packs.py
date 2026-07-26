#!/usr/bin/env python3
"""Gate the bundled language packs against the English locale they translate.

Why this exists: the dashboard bundles English only (`dashboard/src/i18n.ts`)
and ships every other language as an external pack written out by
`install_default_packs()` from `dashboard/src-tauri/resources/*.json`. A key the
pack never got is not an error at runtime — react-i18next silently resolves it
through `fallbackLng: 'en'`. Component tests assert against the English bundle,
so nothing in CI sees it either. The defect only surfaces by rendering the real
GUI in that locale.

It did surface, exactly that way: the 2026-07-27 opverify apex run found the
Danger Zone — the most destructive screen in the app — rendering wholly in
English for a Japanese user, because `settings.health.danger.*` had never been
added to the ja pack (103 of 652 keys missing, an earlier decision). This check is the
part that does not depend on someone looking at a screenshot.

Blocking (exit 1):
  * a key in the English locale that the pack has no translation for
  * an interpolation placeholder set that differs between English and the pack
    (`{{count}}` dropped renders a sentence with a hole in it)
  * a namespace in the pack that `i18n.ts` does not register — it can never load
  * a locale file on disk that `i18n.ts` does not bundle — it can never render
  * a pack missing `code` / `label` (the loader skips such a file entirely)

Advisory (reported, exit 0):
  * keys the pack carries that English no longer has (stale, harmless)
  * values identical to English — legitimate for proper nouns ("ClotoCore"),
    symbols ("—") and format strings ("{{tokens}} tok"), so it cannot block

Plural families (`key_one` / `key_other`, i18next CLDR suffixes) require only
`key_other`: CLDR guarantees that form in every language, and Japanese has no
other. Additional forms are reported as advisory, since which ones a language
needs is a property of the language.

Usage:
  python3 scripts/check-language-packs.py            # check the repo
  python3 scripts/check-language-packs.py --selftest # prove the gate still bites
"""

import argparse
import contextlib
import io
import json
import re
import sys
import tempfile
from pathlib import Path

EN_DIR = Path("dashboard/src/locales/en")
PACK_DIR = Path("dashboard/src-tauri/resources")
I18N_TS = Path("dashboard/src/i18n.ts")

# i18next CLDR plural suffixes. `other` is the form every language has.
PLURAL_SUFFIXES = ("zero", "one", "two", "few", "many", "other")
PLACEHOLDER_RE = re.compile(r"\{\{\s*([^{},\s]+)")
NAMESPACES_RE = re.compile(r"const NAMESPACES = \[(.*?)\]", re.DOTALL)
PACK_META_KEYS = ("code", "label")


def flatten(obj, prefix=""):
    """Flatten a nested locale object into {dotted.key: value}."""
    out = {}
    for key, value in obj.items():
        here = f"{prefix}.{key}" if prefix else key
        if isinstance(value, dict):
            out.update(flatten(value, here))
        else:
            out[here] = value
    return out


def placeholders(value):
    """The interpolation variable names in a locale string."""
    if not isinstance(value, str):
        return frozenset()
    return frozenset(PLACEHOLDER_RE.findall(value))


def split_plural(key):
    """('a.b_one') -> ('a.b', 'one'); a non-plural key -> (key, None)."""
    for suffix in PLURAL_SUFFIXES:
        tail = f"_{suffix}"
        if key.endswith(tail) and len(key) > len(tail):
            return key[: -len(tail)], suffix
    return key, None


def plural_families(keys):
    """Base keys that appear with more than one CLDR suffix in the reference."""
    seen = {}
    for key in keys:
        base, suffix = split_plural(key)
        if suffix:
            seen.setdefault(base, set()).add(suffix)
    return {base for base, suffixes in seen.items() if suffixes}


def load_reference(en_dir):
    """The English locale, as {namespace.dotted.key: value}."""
    reference = {}
    for path in sorted(en_dir.glob("*.json")):
        namespace = path.stem
        data = json.loads(path.read_text(encoding="utf-8"))
        for key, value in flatten(data).items():
            reference[f"{namespace}.{key}"] = value
    return reference


def bundled_namespaces(i18n_ts):
    """The namespaces i18n.ts registers, in declaration order."""
    match = NAMESPACES_RE.search(i18n_ts.read_text(encoding="utf-8"))
    if not match:
        return None
    return [name for name in re.findall(r"'([^']+)'", match.group(1))]


def required_keys(reference):
    """Reference keys a pack must translate, with plural families reduced to `_other`."""
    families = plural_families(reference)
    required = set()
    for key in reference:
        base, suffix = split_plural(key)
        if suffix and base in families:
            required.add(f"{base}_other")
        else:
            required.add(key)
    return required, families


def reference_for(key, reference):
    """The English value a pack key is checked against."""
    if key in reference:
        return reference[key]
    base, suffix = split_plural(key)
    if suffix:
        for candidate in (f"{base}_other", f"{base}_one"):
            if candidate in reference:
                return reference[candidate]
    return None


def check_pack(path, reference, required, families, namespaces):
    """Check one pack. Returns (errors, warnings) as lists of strings."""
    errors = []
    warnings = []
    label = path.name

    try:
        pack = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        return [f"{label}: not valid JSON — the loader skips it entirely ({exc})"], []

    for meta in PACK_META_KEYS:
        if not isinstance(pack.get(meta), str) or not pack[meta].strip():
            errors.append(f"{label}: missing `{meta}` — the loader skips packs without it")

    if namespaces is not None:
        unknown = sorted(
            key
            for key, value in pack.items()
            if isinstance(value, dict) and key not in namespaces
        )
        for name in unknown:
            errors.append(
                f"{label}: namespace `{name}` is not registered in i18n.ts — it can never load"
            )

    flat = {}
    for key, value in pack.items():
        if isinstance(value, dict):
            for sub, leaf in flatten(value, key).items():
                flat[sub] = leaf

    missing = sorted(required - flat.keys())
    for key in missing:
        errors.append(f"{label}: missing `{key}` — renders in English via fallbackLng")

    for key, value in sorted(flat.items()):
        english = reference_for(key, reference)
        if english is None:
            warnings.append(f"{label}: `{key}` is not in the English locale (stale)")
            continue
        mine, theirs = placeholders(value), placeholders(english)
        if mine != theirs:
            lost = ", ".join(sorted(theirs - mine)) or "-"
            extra = ", ".join(sorted(mine - theirs)) or "-"
            errors.append(
                f"{label}: `{key}` placeholder drift — dropped: {lost} / unknown: {extra}"
            )
        elif value == english:
            warnings.append(f"{label}: `{key}` is identical to English")

    for base in sorted(families):
        present = {
            suffix for suffix in PLURAL_SUFFIXES if f"{base}_{suffix}" in flat
        }
        absent = sorted(
            suffix
            for suffix in PLURAL_SUFFIXES
            if f"{base}_{suffix}" in reference and suffix not in present
        )
        if absent:
            warnings.append(
                f"{label}: plural `{base}` has no {', '.join(absent)} form "
                "(only `other` is required)"
            )

    return errors, warnings


def run(root, verbose):
    """Check every pack under `root`. Returns the process exit code."""
    en_dir, pack_dir, i18n_ts = root / EN_DIR, root / PACK_DIR, root / I18N_TS

    if not en_dir.is_dir():
        print(f"ERROR: no English locale at {en_dir}", file=sys.stderr)
        return 1

    reference = load_reference(en_dir)
    if not reference:
        print(f"ERROR: English locale at {en_dir} is empty", file=sys.stderr)
        return 1

    errors, warnings = [], []
    namespaces = None
    if i18n_ts.is_file():
        namespaces = bundled_namespaces(i18n_ts)
        if namespaces is None:
            errors.append(
                "i18n.ts: could not read the NAMESPACES array — "
                "namespace coverage is unchecked"
            )
        else:
            unbundled = sorted({path.stem for path in en_dir.glob("*.json")} - set(namespaces))
            for name in unbundled:
                errors.append(
                    f"locales/en/{name}.json is not in i18n.ts NAMESPACES — it can never render"
                )

    required, families = required_keys(reference)
    packs = sorted(p for p in pack_dir.glob("*.json")) if pack_dir.is_dir() else []

    for path in packs:
        pack_errors, pack_warnings = check_pack(path, reference, required, families, namespaces)
        errors.extend(pack_errors)
        warnings.extend(pack_warnings)

    for warning in warnings if verbose else []:
        print(f"warn: {warning}")
    if warnings and not verbose:
        print(f"{len(warnings)} advisory finding(s) — re-run with --verbose to list them")

    for error in errors:
        print(f"FAIL: {error}", file=sys.stderr)

    print(
        f"language packs: {len(packs)} pack(s) against {len(reference)} English key(s) "
        f"({len(required)} required) — {len(errors)} error(s), {len(warnings)} advisory"
    )
    return 1 if errors else 0


# ── selftest ──────────────────────────────────────────────────────────────


def _write(root, rel, obj):
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        obj if isinstance(obj, str) else json.dumps(obj, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )


def _quiet_run(root):
    """`run` with its report swallowed — the selftest reports its own verdict."""
    sink = io.StringIO()
    with contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
        return run(root, verbose=False)


def _fixture(root, pack):
    """A minimal repo shape: two namespaces, one pack, i18n.ts registering both."""
    _write(root, EN_DIR / "nav.json", {"settings": "Settings", "count_one": "{{count}} item", "count_other": "{{count}} items"})
    _write(root, EN_DIR / "settings.json", {"danger": {"title": "Danger Zone", "meta": "v{{version}}"}})
    _write(root, I18N_TS, "const NAMESPACES = [\n  'nav',\n  'settings',\n] as const;\n")
    _write(root, PACK_DIR / "ja.json", pack)


def _base_pack():
    return {
        "code": "ja",
        "label": "日本語",
        "nav": {"settings": "設定", "count_other": "{{count}} 件"},
        "settings": {"danger": {"title": "危険な操作", "meta": "v{{version}}"}},
    }


def selftest():
    """Prove each blocking rule actually fails, and each advisory does not."""
    cases = []

    def case(name, expected, mutate):
        cases.append((name, expected, mutate))

    case("clean pack passes", 0, lambda p: p)

    def drop_key(pack):
        del pack["settings"]["danger"]["title"]
        return pack

    case("missing key fails", 1, drop_key)

    def drop_placeholder(pack):
        pack["settings"]["danger"]["meta"] = "バージョン"
        return pack

    case("placeholder drift fails", 1, drop_placeholder)

    def unknown_namespace(pack):
        pack["mystery"] = {"a": "b"}
        return pack

    case("unregistered namespace fails", 1, unknown_namespace)

    def drop_label(pack):
        del pack["label"]
        return pack

    case("missing label fails", 1, drop_label)

    def stale_key(pack):
        pack["nav"]["gone"] = "消えたキー"
        return pack

    case("stale key is advisory only", 0, stale_key)

    def identical_value(pack):
        pack["nav"]["settings"] = "Settings"
        return pack

    case("value identical to English is advisory only", 0, identical_value)

    def only_other_plural(pack):
        return pack  # base pack already omits `count_one`

    case("plural family needs only `other`", 0, only_other_plural)

    failures = []
    for name, expected, mutate in cases:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _fixture(root, mutate(_base_pack()))
            actual = _quiet_run(root)
            if actual != expected:
                failures.append(f"{name}: expected exit {expected}, got {actual}")

    # A pack file that is not valid JSON must fail rather than be skipped.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        _fixture(root, _base_pack())
        _write(root, PACK_DIR / "broken.json", "{ not json")
        if _quiet_run(root) != 1:
            failures.append("invalid JSON pack: expected exit 1")

    # A locale file that i18n.ts never bundles can never render.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        _fixture(root, _base_pack())
        _write(root, EN_DIR / "orphan.json", {"a": "b"})
        if _quiet_run(root) != 1:
            failures.append("unbundled locale file: expected exit 1")

    if failures:
        for failure in failures:
            print(f"selftest FAIL: {failure}", file=sys.stderr)
        return 1
    print(f"selftest: OK ({len(cases) + 2} cases)")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root (default: .)")
    parser.add_argument("--verbose", action="store_true", help="list advisory findings")
    parser.add_argument("--selftest", action="store_true", help="check the checker")
    args = parser.parse_args()

    if args.selftest:
        return selftest()
    return run(Path(args.root), args.verbose)


if __name__ == "__main__":
    sys.exit(main())
