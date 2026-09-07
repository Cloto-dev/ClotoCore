# Issue registry schema

`qa/issue-registry.json` is the bug ledger. It is not a record of past work:
`scripts/verify-issues.sh` reads every entry on each CI run and greps the named file
for the named pattern, so an entry that no longer describes the tree fails the build.
That is the point — the registry is a machine-checked claim about the code, and this
file describes the shape of one claim.

This document is named by the registry's own `$schema` field, and the verifier refuses
a registry whose `$schema` does not resolve to a file in this repository. The pointer
had been dangling for months before anyone noticed, because nothing read it.

## File shape

```json
{
  "$schema": "qa/issue-registry.schema.md",
  "description": "...",
  "issues": [ { ... }, { ... } ]
}
```

There is no schema version number. The registry has one producer and one consumer,
both in this repository and versioned together by git, so there is no version skew for
such a field to describe — and a field nothing reads states nothing. What the shape is
today is what this document says it is.

## Entry fields

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | `bug-NNN`, permanent and never reused. The canonical identifier for the defect everywhere in the tree — code comments, fix markers, test names and registry patterns refer to a defect by this id and no other. Ids are local to this repository: a `bug-NNN` here and the same number in a sibling registry are unrelated. |
| `summary` | yes | What is wrong, in enough detail to judge severity without opening the file. |
| `severity` | yes | `CRITICAL` / `HIGH` / `MEDIUM` / `LOW`. |
| `discovered` | yes | ISO-8601 timestamp. |
| `version` | yes | The version the defect was found in. Per entry — unrelated to the file shape. |
| `file` | yes | Repo-relative path the pattern is checked against. **Follows the anchor**, not the defect's history — when a fix lands in a different file than the report named, move this with it. |
| `pattern` | yes | A `grep -P` (falling back to `-E`) regular expression. Never empty: an empty pattern matches every line, which the verifier refuses rather than reporting as proof. |
| `expected` | yes | `present` or `absent`. See below. |
| `status` | yes | `open`, `fixed`, `wontfix`, or `obsolete`. `obsolete` entries are skipped entirely; `wontfix` entries are verified like any other, because a defect nobody intends to fix still has to still be there. |
| `commit` | no | The commit that introduced the defect, where known. |
| `github_issue` | no | The GitHub issue number this entry is mirrored to. Written by the sync workflow, not by hand. |
| `notes` | no | Anything else — how it was found, constraints, cross-references to related ids. |
| `fix_note` | no | What the fix does **and why that fix rather than another**. The rejected alternative is worth more than the diff, which git already has. |
| `fixed_in` | no | The version the fix landed in. |
| `advisory` | no | For a defect that needs to reach users of released versions: `{ "affected": "<version range>", "symptom_check": "<probe id>" }`. Read by the release-feed generator. |

No field may contain a newline or an ASCII unit separator (`\x1f`); the verifier
serialises records with `\x1f` and aborts loudly rather than silently dropping a row.

## `expected`, and how it changes across a fix

`expected` says what the verifier should find, so it encodes which side of the fix the
pattern describes.

- **`present`** — the pattern must match. For an `open` entry this anchors the *defect*
  (proof it is still there). For a `fixed` entry it anchors the *fix* (proof it has not
  been reverted), which is why a `bug-NNN` marker in a comment makes a good pattern.
- **`absent`** — the pattern must NOT match. Used for a `fixed` entry whose fix
  *removed* the offending construct. A file that no longer exists counts as absent.

A missing file with `expected: present` is an `ERROR`. A `present` pattern that no
longer matches is `STALE`. An `absent` pattern that still matches is `UNFIXED`. Any of
the three fails the gate.

**Fixing a registered bug therefore means editing its entry in the same change**: flip
`status`, add `fixed_in` and `fix_note`, and re-anchor `pattern`/`expected`/`file` on
whatever now proves the fix.

## Choosing a pattern

The pattern is the whole mechanism, and two failure modes are easy to walk into:

1. **Not unique.** A string that also occurs elsewhere in the same file can never go
   `absent`. Check with `grep -c` before writing the entry — if the construct you
   removed appears three more times legitimately, anchor on something else the fix
   actually deleted.
2. **Not stable.** Prefer a construct the fix demonstrably changes (a `bug-NNN` marker,
   a renamed helper, a distinctive literal) over incidental formatting or a line the
   next refactor will move for unrelated reasons. Line numbers are never part of a
   pattern.

Escape regex metacharacters (`.` `(` `)` `*` `[`) — the pattern goes to `grep -P`, not
to a literal string comparison.

## What the verifier refuses

Every one of these was once a way for an entry to leave the run with a verdict nothing
produced, while the gate exited 0. The callers read only the exit code, so a green run
is the whole report and a row that was never checked is indistinguishable from a row
that passed. Each is now a named failure:

- a registry that does not parse, or that carries no `issues` list
- a registry that declares zero entries — an empty ledger verifies nothing
- a `$schema` that is absent, or that does not resolve to a file in this repository
- a field containing the record separator, a newline or a carriage return
- an empty `pattern`, which `grep -c` matches against every line
- an `expected` that is neither `present` nor `absent`
- a mismatch between the rows emitted and the rows read, or between the rows counted
  and the four verdict buckets

## Sibling registries

Other repositories in this project carry registries built on the same idea, with
verifiers grown from this one. They are **not identical** and are not kept in sync:
each has hardened against defects the others had not hit, and each carries fields the
others do not. Do not copy an entry between them expecting it to validate, and do not
assume a guard present here exists there. Each registry names its own schema document.

## Running it

```bash
bash scripts/verify-issues.sh              # everything
bash scripts/verify-issues.sh --filter open
```

Exit code 0 means every entry still describes the tree. `scripts/verify-issues.sh` is
read-only verification infrastructure: never modify the script to make a check pass.
