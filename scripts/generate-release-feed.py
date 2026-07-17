#!/usr/bin/env python3
"""Generate the ClotoCore updater feed (manifest.json + channel views).

The feed is a pure function of (GitHub Releases state, .release/lifecycle.json):
every run rebuilds it from scratch — there is no stored feed state to migrate
or corrupt. Design authority: docs/RELEASE_PIPELINE_DESIGN.md §4.

Outputs (written to --out):
  manifest.json        master manifest (all releases, tiers derived)
  stable.json          Tauri-updater-format view for the stable channel
  current.json         …for the current channel
  experimental.json    …for the experimental channel

Channel resolution (RELEASE_PIPELINE_DESIGN.md §3):
  experimental  newest release including pre-releases
  current       newest final release
  stable        newest final release of lifecycle.stable_line;
                while stable_line is null, aliases current (initial-state
                rule, Release Lifecycle Standard v1.1 §2.6) and the view
                carries "stable_alias": true.

Requires the `gh` CLI (authenticated in CI via GITHUB_TOKEN) for release
listing; asset payloads are fetched over plain HTTPS.
"""

import argparse
import json
import re
import subprocess
import sys
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

FEED_TAG = "updater-feed"
SEMVER_RE = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|rc)\.(\d+))?$")
STAGE_RANK = {"alpha": 0, "beta": 1, "rc": 2}
KERNEL_RE = re.compile(r"^cloto-.+-((?:linux|macos|windows)-(?:x64|arm64))\.(?:tar\.gz|zip)$")


def log(msg: str) -> None:
    print(msg, file=sys.stderr)


def parse_version(tag: str):
    """Parse a (possibly v-prefixed) semver tag into a sortable key.

    Key shape: (major, minor, patch, is_final, stage_rank, stage_num) —
    a final release sorts above every pre-release of the same version,
    matching semver precedence for the grammar this repo uses.
    Returns None for tags outside that grammar.
    """
    m = SEMVER_RE.match(tag)
    if not m:
        return None
    major, minor, patch = int(m[1]), int(m[2]), int(m[3])
    if m[4] is None:
        return (major, minor, patch, 1, 0, 0)
    return (major, minor, patch, 0, STAGE_RANK[m[4]], int(m[5]))


def is_final(key) -> bool:
    return key[3] == 1


def line_of(key) -> str:
    return f"{key[0]}.{key[1]}"


def tier_of(key, stable_line) -> str:
    if not is_final(key):
        return "experimental"
    if stable_line is not None and line_of(key) == stable_line:
        return "stable"
    return "current"


def desktop_platform(asset_name: str):
    """Map an updater artifact filename to its Tauri platform key.

    Mirrors the globs of the per-release latest.json generation step in
    .github/workflows/release.yml — keep the two in sync.
    """
    if asset_name.endswith("_x64-setup.nsis.zip"):
        return "windows-x86_64"
    if asset_name.endswith(".AppImage"):
        return "linux-x86_64"
    if asset_name.endswith(".app.tar.gz"):
        if "aarch64" in asset_name:
            return "darwin-aarch64"
        if "x64" in asset_name:
            return "darwin-x86_64"
    return None


def kernel_platform(asset_name: str):
    m = KERNEL_RE.match(asset_name)
    return m[1] if m else None


def advisories_from(issues: list) -> list:
    """Extract curated advisories (DEFENDER_DESIGN.md §5) from issue-registry
    entries. Opt-in: only entries carrying an explicit `advisory` object are
    exported — mechanically converting every fixed bug would drown the client
    in noise. The advisory object supplies `affected` (version range) and
    optionally `symptom_check` (a defender registry check name that
    corroborates the issue locally)."""
    out = []
    for issue in issues:
        adv = issue.get("advisory")
        if not adv:
            continue
        if not adv.get("affected"):
            log(f"  warn: {issue.get('id')}: advisory without 'affected' range — skipped")
            continue
        out.append(
            {
                "bug_id": issue.get("id"),
                "severity": (issue.get("severity") or "").lower(),
                "affected": adv["affected"],
                "fixed_in": issue.get("fixed_in"),
                "symptom_check": adv.get("symptom_check"),
                "summary": issue.get("summary", ""),
            }
        )
    return out


def load_advisories(path: Path) -> list:
    if not path.exists():
        log(f"  warn: issue registry not found at {path} — advisories block will be empty")
        return []
    data = json.loads(path.read_text())
    return advisories_from(data.get("issues", []))


def http_get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "clotocore-feed-generator"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return resp.read()


def list_releases(repo: str):
    """List all non-draft releases via the gh CLI (handles auth + pagination)."""
    releases = []
    page = 1
    while True:
        out = subprocess.run(
            ["gh", "api", f"repos/{repo}/releases?per_page=100&page={page}"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        batch = json.loads(out)
        if not batch:
            return releases
        releases.extend(r for r in batch if not r.get("draft"))
        page += 1


def build_entry(release) -> dict | None:
    """Build one manifest entry; fetches SHA256SUMS.txt and .sig assets."""
    tag = release["tag_name"]
    key = parse_version(tag)
    assets = {a["name"]: a["browser_download_url"] for a in release.get("assets", [])}

    sha256 = {}
    if "SHA256SUMS.txt" in assets:
        try:
            for line in http_get(assets["SHA256SUMS.txt"]).decode().splitlines():
                parts = line.split()
                if len(parts) == 2:
                    sha256[parts[1].lstrip("*")] = parts[0]
        except Exception as e:  # noqa: BLE001 — a missing sums file degrades, not aborts
            log(f"  warn: {tag}: SHA256SUMS.txt fetch failed: {e}")

    def sig_for(name: str):
        sig_url = assets.get(f"{name}.sig")
        if not sig_url:
            return None
        try:
            return http_get(sig_url).decode().strip()
        except Exception as e:  # noqa: BLE001
            log(f"  warn: {tag}: {name}.sig fetch failed: {e}")
            return None

    desktop, kernel = {}, {}
    for name, url in assets.items():
        dp = desktop_platform(name)
        if dp and dp not in desktop:
            desktop[dp] = {"url": url, "sha256": sha256.get(name), "signature": sig_for(name)}
            continue
        kp = kernel_platform(name)
        if kp and kp not in kernel:
            kernel[kp] = {"url": url, "sha256": sha256.get(name)}

    if not desktop and not kernel:
        log(f"  skip: {tag}: no distributable artifacts")
        return None

    return {
        "version": tag.lstrip("v"),
        "_key": key,
        "tier": None,  # filled in after lifecycle is applied
        "pub_date": release.get("published_at"),
        "notes_url": release.get("html_url"),
        "desktop": desktop,
        "kernel": kernel,
    }


def resolve_channel(entries, channel: str, stable_line):
    """Pick the newest entry visible on `channel` that has usable desktop
    artifacts (a view exists for the updater; signature-less platforms are
    dropped from views but kept in the manifest)."""

    def visible(e):
        if channel == "experimental":
            return True
        if channel == "current":
            return is_final(e["_key"])
        # stable: alias current while no line is certified (§2.6)
        if stable_line is None:
            return is_final(e["_key"])
        return is_final(e["_key"]) and line_of(e["_key"]) == stable_line

    candidates = [e for e in entries if visible(e) and any(p["signature"] for p in e["desktop"].values())]
    return max(candidates, key=lambda e: e["_key"], default=None)


def channel_view(entry, *, stable_alias: bool = False) -> dict:
    view = {
        "version": entry["version"],
        "notes": f"ClotoCore v{entry['version']} — {entry['notes_url']}",
        "pub_date": entry["pub_date"],
        "platforms": {
            platform: {"signature": info["signature"], "url": info["url"]}
            for platform, info in entry["desktop"].items()
            if info["signature"]
        },
    }
    if stable_alias:
        view["stable_alias"] = True
    return view


def generate(
    repo: str, lifecycle: dict, out_dir: Path, max_workers: int, advisories: list | None = None
) -> None:
    stable_line = lifecycle.get("stable_line")

    releases = [
        r
        for r in list_releases(repo)
        if r["tag_name"] != FEED_TAG and parse_version(r["tag_name"]) is not None
    ]
    log(f"{len(releases)} releases to index")

    with ThreadPoolExecutor(max_workers=max_workers) as pool:
        entries = [e for e in pool.map(build_entry, releases) if e]
    entries.sort(key=lambda e: e["_key"], reverse=True)
    for e in entries:
        e["tier"] = tier_of(e["_key"], stable_line)

    out_dir.mkdir(parents=True, exist_ok=True)

    views = {}
    for channel in ("stable", "current", "experimental"):
        chosen = resolve_channel(entries, channel, stable_line)
        if chosen is None:
            log(f"  warn: no resolvable release for channel '{channel}' — view not written")
            continue
        views[channel] = channel_view(chosen, stable_alias=(channel == "stable" and stable_line is None))
        (out_dir / f"{channel}.json").write_text(json.dumps(views[channel], indent=2) + "\n")
        log(f"  {channel}: {chosen['version']} ({len(views[channel]['platforms'])} platforms)")

    manifest = {
        "schema_version": 1,
        "generated_at": None,  # stamped by the workflow (kept out of the pure function)
        "lifecycle": lifecycle,
        "channels": {name: view["version"] for name, view in views.items()},
        "releases": [{k: v for k, v in e.items() if k != "_key"} for e in entries],
        # Known-issue advisories for the defender (DEFENDER_DESIGN.md §5) —
        # additive field, consumed by `clotocore doctor` / health scan.
        "advisories": advisories or [],
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    log(f"wrote {out_dir}/manifest.json ({len(entries)} releases)")


def selftest() -> None:
    # semver parsing + precedence
    assert parse_version("v1.2.3") == (1, 2, 3, 1, 0, 0)
    assert parse_version("0.6.8-beta.1") == (0, 6, 8, 0, 1, 1)
    assert parse_version("v0.6.8-rc.2") == (0, 6, 8, 0, 2, 2)
    assert parse_version("updater-feed") is None
    assert parse_version("v1.2") is None
    order = ["v0.6.7", "v0.6.8-alpha.1", "v0.6.8-alpha.2", "v0.6.8-beta.1", "v0.6.8-rc.1", "v0.6.8", "v0.7.0-alpha.1"]
    keys = [parse_version(t) for t in order]
    assert keys == sorted(keys), "semver precedence ladder broken"

    # tier derivation
    assert tier_of(parse_version("v0.6.8-beta.1"), None) == "experimental"
    assert tier_of(parse_version("v0.6.7"), None) == "current"
    assert tier_of(parse_version("v0.6.7"), "0.6") == "stable"
    assert tier_of(parse_version("v0.7.0"), "0.6") == "current"

    # platform mapping (mirrors release.yml globs)
    assert desktop_platform("ClotoCore_0.6.7_x64-setup.nsis.zip") == "windows-x86_64"
    assert desktop_platform("ClotoCore_0.6.7_amd64.AppImage") == "linux-x86_64"
    assert desktop_platform("ClotoCore_aarch64.app.tar.gz") == "darwin-aarch64"
    assert desktop_platform("ClotoCore_x64.app.tar.gz") == "darwin-x86_64"
    assert desktop_platform("cloto-0.6.7-linux-x64.tar.gz") is None
    assert kernel_platform("cloto-0.6.7-linux-x64.tar.gz") == "linux-x64"
    assert kernel_platform("cloto-0.6.7-windows-x64.zip") == "windows-x64"
    assert kernel_platform("clotocore-x86_64-unknown-linux-gnu") is None

    # channel resolution incl. the §2.6 stable→current alias
    def entry(tag, sig=True):
        key = parse_version(tag)
        return {
            "version": tag.lstrip("v"),
            "_key": key,
            "pub_date": "2026-01-01T00:00:00Z",
            "notes_url": "https://example.invalid",
            "desktop": {"linux-x86_64": {"url": "u", "sha256": None, "signature": "s" if sig else None}},
            "kernel": {},
        }

    entries = [entry(t) for t in ("v0.6.7", "v0.6.8-beta.1", "v0.6.4")]
    assert resolve_channel(entries, "experimental", None)["version"] == "0.6.8-beta.1"
    assert resolve_channel(entries, "current", None)["version"] == "0.6.7"
    assert resolve_channel(entries, "stable", None)["version"] == "0.6.7"  # alias
    assert resolve_channel(entries, "stable", "0.6")["version"] == "0.6.7"
    with_070 = entries + [entry("v0.7.0")]
    assert resolve_channel(with_070, "stable", "0.6")["version"] == "0.6.7"  # pinned line
    assert resolve_channel(with_070, "current", "0.6")["version"] == "0.7.0"
    # a release whose desktop artifacts are unsigned is not view-eligible
    unsigned_newest = entries + [entry("v0.7.0", sig=False)]
    assert resolve_channel(unsigned_newest, "current", None)["version"] == "0.6.7"
    # alias marking
    view = channel_view(entry("v0.6.7"), stable_alias=True)
    assert view["stable_alias"] is True and "linux-x86_64" in view["platforms"]

    # advisory extraction: opt-in via the `advisory` object, range required
    issues = [
        {
            "id": "bug-386",
            "summary": "legacy install breaks boot",
            "severity": "CRITICAL",
            "status": "fixed",
            "fixed_in": "0.6.7",
            "advisory": {"affected": ">=0.6.0 <0.6.7", "symptom_check": "legacy_data_dir_drift"},
        },
        {"id": "bug-001", "summary": "no advisory object", "severity": "HIGH", "status": "fixed"},
        {"id": "bug-002", "summary": "advisory without range", "advisory": {}},
    ]
    advisories = advisories_from(issues)
    assert len(advisories) == 1, "only entries with a complete advisory object are exported"
    assert advisories[0] == {
        "bug_id": "bug-386",
        "severity": "critical",
        "affected": ">=0.6.0 <0.6.7",
        "fixed_in": "0.6.7",
        "symptom_check": "legacy_data_dir_drift",
        "summary": "legacy install breaks boot",
    }

    print("selftest: OK")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--repo", help="owner/name (required unless --selftest)")
    ap.add_argument("--lifecycle", default=".release/lifecycle.json")
    ap.add_argument("--issue-registry", default="qa/issue-registry.json")
    ap.add_argument("--out", default="feed")
    ap.add_argument("--max-workers", type=int, default=8)
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        selftest()
        return 0
    if not args.repo:
        ap.error("--repo is required")

    lifecycle = json.loads(Path(args.lifecycle).read_text())
    advisories = load_advisories(Path(args.issue_registry))
    generate(args.repo, lifecycle, Path(args.out), args.max_workers, advisories)
    return 0


if __name__ == "__main__":
    sys.exit(main())
