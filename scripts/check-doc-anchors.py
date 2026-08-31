#!/usr/bin/env python3
"""Verify that every in-site link lands on a page that exists, at an id that exists.

This covers two classes, both of which `mkdocs build --strict` lets through.

**Anchors.** --strict fails on a broken *page* link but says nothing about a
broken *#fragment*: a link to `ARCHITECTURE.md#event-bus` keeps passing after
the heading it names is gone. (`validation.links.anchors` catches the Markdown
sources; it does not see fragments written as raw HTML, nor anchors on a page
whose headings moved after the link was written.) Translations make the gap
load-bearing: a `.ja.md` page replaces the English fallback at `/ja/<page>/`
and derives its anchors from its own headings, so translating a heading
invalidates every cross-page link aimed at the English slug — on the Japanese
site only. That is why translated pages carry explicit heading ids
(`## 見出し { #english-slug }`) and why this check exists to prove they do.

**Links into the unpublished set.** `docs/` holds documents that are
deliberately not published (see `docs/DOCUMENTATION_POLICY.md`), listed in
`exclude_docs`. A published page that links to one of them renders a relative
href to a page that was never built — a 404 for the reader. mkdocs reports
this, but at a severity it caps itself:

    warning_level = min(logging.INFO, self.config.validation.links.not_found)

so no `validation` setting can raise it to a warning and --strict can never
fail on it. Measured on this repository while the site was being set up: three
such links in ARCHITECTURE.md, build green. Since the gate cannot be
configured to exist, it lives here instead.

This runs against the *built* site rather than the Markdown sources, so it sees
what a reader's browser sees: fallback pages, theme-generated links, and
percent-encoded fragments included.

Usage: check-doc-anchors.py [site_dir]   (default: ./site)
Exit 1 on any unresolved target or fragment.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit

HREF = re.compile(r'href="([^"]+)"')
# id="..." covers headings and theme anchors; name="..." covers legacy targets.
ID = re.compile(r'\b(?:id|name)="([^"]+)"')
# Note "#" is deliberately absent: a same-page fragment link is the most
# common kind on the site (every table-of-contents entry is one), and
# skipping it leaves the anchor arm of this check with nothing to check.
EXTERNAL = ("http://", "https://", "mailto:", "tel:", "data:", "//")


def ids_of(html_path: Path, cache: dict[Path, set[str]]) -> set[str]:
    if html_path not in cache:
        try:
            cache[html_path] = set(ID.findall(html_path.read_text(errors="replace")))
        except OSError:
            cache[html_path] = set()
    return cache[html_path]


def base_path(site: Path) -> str:
    """The URL prefix the site is deployed under, e.g. '/ClotoCore/'.

    Absolute hrefs are written against the deployed root, not against the local
    output directory — `404.html` uses them exclusively, because it is served
    from whatever path the reader mistyped and cannot use relative links. Read
    the prefix off the generated sitemap rather than re-parsing mkdocs.yml, so
    the checker agrees with what was actually built.
    """
    sitemap = site / "sitemap.xml"
    if not sitemap.is_file():
        return "/"
    match = re.search(r"<loc>\s*([^<\s]+)", sitemap.read_text(errors="replace"))
    if not match:
        return "/"
    path = urlsplit(match.group(1)).path or "/"
    return path if path.endswith("/") else path.rsplit("/", 1)[0] + "/"


def resolve(site: Path, page: Path, path_part: str, base: str) -> Path:
    """Map an href's path component to the file a browser would fetch."""
    if path_part.startswith("/"):
        rooted = path_part
        if base != "/" and rooted.startswith(base):
            rooted = rooted[len(base) :]
        target = site / rooted.lstrip("/")
    else:
        target = page.parent / path_part
    target = target.resolve()
    if target.is_dir() or path_part.endswith("/") or not target.suffix:
        candidate = target / "index.html"
        if candidate.is_file() or target.is_dir():
            return candidate
        return target.with_suffix(".html") if not target.suffix else target
    return target


def main() -> int:
    site = Path(sys.argv[1] if len(sys.argv) > 1 else "site").resolve()
    if not site.is_dir():
        print(f"site directory not found: {site}", file=sys.stderr)
        return 2

    pages = sorted(site.rglob("*.html"))
    if not pages:
        print(f"no HTML found under {site} — did the build run?", file=sys.stderr)
        return 2

    base = base_path(site)
    cache: dict[Path, set[str]] = {}
    missing: list[str] = []
    anchors: list[str] = []
    links_checked = 0
    fragments_checked = 0

    for page in pages:
        for href in HREF.findall(page.read_text(errors="replace")):
            if href.startswith(EXTERNAL):
                continue
            path_part, _, fragment = href.partition("#")
            fragment = unquote(urlsplit(fragment).path or fragment)

            if not path_part:
                target = page
            else:
                target = resolve(site, page, path_part.split("?")[0], base)
                # Only judge targets inside the built site; a link that walks
                # out of it is a different question and not this one's.
                try:
                    target.relative_to(site)
                except ValueError:
                    continue
                links_checked += 1
                if not target.is_file():
                    missing.append(
                        f"{page.relative_to(site)} -> {path_part}"
                        f" (no {target.relative_to(site)} in the built site)"
                    )
                    continue

            if not fragment:
                continue  # a page link with no fragment makes no anchor claim

            fragments_checked += 1
            if fragment not in ids_of(target, cache):
                anchors.append(
                    f"{page.relative_to(site)} -> {target.relative_to(site)}#{fragment}"
                )

    for kind, findings in (("link", missing), ("anchor", anchors)):
        for f in sorted(set(findings)):
            print(f"::error::broken {kind}: {f}")
            print(f"  - broken {kind}: {f}", file=sys.stderr)

    if missing or anchors:
        print(
            f"{len(set(missing))} broken link(s) out of {links_checked} checked, "
            f"{len(set(anchors))} broken anchor(s) out of {fragments_checked} checked",
            file=sys.stderr,
        )
        return 1

    print(
        f"doc links: OK ({links_checked} in-site links and "
        f"{fragments_checked} fragments resolve)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
