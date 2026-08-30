# Documentation Policy

What belongs on the public documentation site, what stays in the repository,
and what gets archived — and how to tell which is which without guessing.

## 1. Why this exists

`docs/` grew to 31 files and 578,379 characters, most of it design notes
written for whoever was building the thing at the time. Publishing that as-is
would have meant maintaining — and, once the site is bilingual, translating —
a corpus almost three times the size of the one the memory server publishes,
most of which no outside reader has a use for.

The first pass on that corpus removed 26.8% of it. This page records the rules
that pass used, because the expensive part was not the moving; it was learning
which documents only *looked* dead.

## 2. Four buckets

| Bucket | Meaning |
| --- | --- |
| **Public** | On the documentation site. Written for someone who does not work on this codebase. |
| **Repository** | Stays in `docs/`, not on the site. Design and contract notes the implementation refers to. |
| **Archive** | Moved to `docs/archive/`. Kept for history; nothing points at it any more. |
| **Elsewhere** | The subject belongs to another repository. Link out rather than keep a copy. |

"Public" is decided by the site navigation, not by where the file sits. A file
in `docs/` that is not in the navigation is not published and carries no
translation obligation. Moving files between repositories is a separate
question from deciding what is published, and a much more expensive one — see
§5.

## 3. Deciding whether a document is dead

**Do not use the last-modified date.** A pointer, a settled contract and an
abandoned plan are indistinguishable by age, and the mistake is not symmetric:
archiving something live breaks the readers who depend on it, while keeping
something dead costs only shelf space.

The document that proved this is the specification stub in this directory. It
had not been touched in three months and would have been archived on age. Its
own text says why it exists: it redirects to the repository that owns the
specification and keeps the contributor guide's required-reading paths
working. **A pointer does not need updating when its target moves, so being
old is what correct looks like for it.**

Ask these instead:

1. **Is it a pointer?** If the authority lives elsewhere and this document
   exists to send readers there, it is alive regardless of how still it is.
2. **Does the implementation cite it?** A document named in a migration, a
   source comment or a component is load-bearing. Where the implementation
   and the document's own status line disagree, **the implementation is the
   evidence and the status line is the stale part** — three documents here
   still say "proposed" for work that shipped.
3. **Would anyone look for it?** One-off audits and point-in-time measurements
   stop being read once their conclusions land in the code or the conventions.
   That is the ordinary end of a report, not a failure.
4. **What refers to it?** Measure this before removing anything, and fix what
   breaks in the same change. A link in a changelog entry is the exception: it
   records what was true then, and rewriting it to match a move would make the
   record wrong.

Criterion 2 does the most work. Of the documents that looked abandoned by
their own status lines, the ones that migrations and UI components cite by
name were all still in force.

## 4. What goes on the site

The site is for someone who has not read this repository. That excludes most
design documents, which assume the reader is building the subsystem.

Initial navigation:

| Page | Source |
| --- | --- |
| Home | `README.md` |
| Getting Started | New — see §6 |
| Architecture | `ARCHITECTURE.md` |
| Build an MCP/MGP server | `QUICKSTART_MCP_SERVER.md` |
| Protocol specification | External link to the specification's own repository |
| Development | `DEVELOPMENT.md` |
| Changelog | `CHANGELOG.md` — 83,605 characters, so published in split or generated form rather than as one page |
| Project vision | `PROJECT_VISION.md` |

`INSTALLER_DISTRIBUTION.md` is not on this list. It describes the distribution
*strategy*, not what a user does, and it has not tracked the packaging changes
made since. Getting Started replaces the role it was standing in for.

## 5. Documents whose subject belongs to another repository

Some documents here are about the memory server, which has its own repository.
Two of them were archived: their content had stopped at a version thirteen
minor releases back while that repository grew current pages of its own, so
what remained was an older view rather than a second one.

The rest stay where they are, and are simply left out of the site navigation.
Physically relocating them was considered and rejected on measurement: each is
referenced from roughly ten places across four repositories, including the
other project's production source, its *published* documentation **and that
page's translation** — editing one side of a translated pair without the other
trips the drift gate. The benefit is tidiness; the cost is a cross-repository
change with several ways to leave a dangling reference. Excluding them from
the navigation achieves everything the site needs.

Where a document has been archived and the README linked to it, the link is
repointed at the owning repository's own current page rather than at the
archived copy. The README already does this for the protocol specification.

## 6. Getting Started — outline and sources

This page does not exist yet. It is the largest gap in the public set: nothing
currently tells a user how to install, configure, update or remove the
application. Each section below names where its content comes from, so writing
it is assembly and verification rather than invention.

| Section | Draws on | Verified against |
| --- | --- | --- |
| Install | Release assets per platform; the release workflow's build matrix | The published installer, run on a clean machine |
| First run | The onboarding design note | The real first-run screens |
| Connect a reasoning provider | The provider endpoints and the catalog | A provider answering a live request |
| Add an MCP server | The marketplace install path and the hub catalog | An install completing end to end |
| Update | The updater feed: the shipped build subscribes to one channel asset, and the feed generator admits only final versions to it, so pre-releases cannot reach it | The published feed |
| Uninstall | The lifecycle subsystem's uninstall design: cumulative scope tiers and the admin-key gate | The real dialog |

**Every claim about what the user sees must be checkable.** The verification
harness enumerates the application's real controls into a committed inventory;
a sentence that names a control can be checked against that inventory rather
than trusted. This makes Getting Started the page where a facts gate pays for
itself first, and it is the reason the outline fixes its sources now: the
sections that cannot name a source are the sections that would have been
invented.

Write the page after the site structure exists, not before — the structure
decides where it sits and how it is split.
