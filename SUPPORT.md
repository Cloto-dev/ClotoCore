# Support Policy

This document defines the release lifecycle and support policy for ClotoCore.
It is written to be line-agnostic: the same rules apply to every release line
(0.6.x, 0.7.x, ...), so the policy survives line transitions unchanged.

This policy is the operative instance of the
[Release Lifecycle Standard](https://github.com/Cloto-dev/cpersona/blob/master/docs/RELEASE_LIFECYCLE_STANDARD.md)
(piloted in cpersona). ClotoCore adopts it as the second pilot repository
and additionally **enforces the tier rules structurally** in its
distribution pipeline — see
[docs/RELEASE_PIPELINE_DESIGN.md](docs/RELEASE_PIPELINE_DESIGN.md).

## Release tiers

Every release line is in exactly one tier at any time. The tier attaches to
the line (e.g. 0.6.x), not to an individual version.

| Tier | Meaning | Fix policy |
| --- | --- | --- |
| **Stable** | Certified by the maintainer after production soak. Recommended for all users; the default update channel serves this line. | Critical bug fixes, data-loss fixes, and security fixes only (backported at the maintainer's discretion). |
| **Current** | The newest release line. It has passed the full release gate but has not yet earned the production-soak certification. | All bug fixes land here first — this is where development happens. |
| **Experimental** | Alpha / beta (and, when needed, rc) pre-releases. Opt-in only; no guarantees of any kind. | Fixes ship in the next pre-release. |

Naming note: **Current** follows the Node.js release vocabulary — the newest
supported release line, distinct from the production-recommended tier. It is
*not* the BSD `-CURRENT` (an unstable development head); that role is played
by **Experimental** here.

### Pre-releases (Experimental)

- Version strings use semver pre-release notation: `0.6.8-alpha.1`,
  `0.6.8-beta.1`, `0.6.8-rc.1`. Git tags match 1:1 (`v0.6.8-beta.1`).
- ClotoCore cuts pre-releases at the *release* level (including patch
  releases within a line), not only at line boundaries; the stage semantics
  are the same.
- The Experimental tier is opt-in by construction: the desktop updater's
  default channel and `install.sh`'s default resolution never select a
  pre-release, and GitHub marks pre-release tags so they never carry the
  "Latest" badge. Selecting the `experimental` update channel is an
  explicit, warned action.

### Release gate (entry into Current)

A release enters Current only after passing, on the release PR/commit:

- `cargo test` (full Rust suite)
- `cargo clippy -- -D warnings` and `cargo fmt -- --check`
- issue-registry verification (`scripts/verify-issues.sh`)
- full CI green, including the dashboard build

### Promotion to Stable

Promotion is an explicit, event-based maintainer decision — there is no
fixed clock. Guideline: several weeks of production soak with no new
critical or high-severity defects. The soak environment is the maintainer's
production desktop deployment. The certification is recorded in the Status
table below and in `.release/lifecycle.json` (which flips the default
update channel's pin — see the pipeline design doc).

### Grace window

When a successor line is certified Stable, the superseded line keeps its
Stable fix policy (critical / data-loss / security only) for **30 days from
the certification date**, then reaches EOL.

- The clock anchors on the certification event and is **not** reset by
  patch releases inside the window.
- Fixes for issues accepted within the window may ship after it closes.
- If a transition requires a database schema or data migration, the
  maintainer SHOULD extend the grace window before certifying the
  successor.

### EOL

No further fixes. Security fixes after EOL are at the maintainer's sole
discretion and must not be relied upon.

## Status

| Line | Tier | Notes |
| --- | --- | --- |
| 0.6.x | **Current** | Latest release: v0.6.7 <!-- docs-facts: latest-release -->. 0.6.8 pre-releases (0.6.8-beta.6 <!-- docs-facts: latest-prerelease -->) are **Experimental**. |

**No line has been certified Stable yet.** Until the first certification
event, the `stable` update channel aliases `current` (the initial-state rule
of the pipeline design, §3.1). Certification and EOL dates are recorded in
this table as they occur.

*Last updated: 2026-07-12*
