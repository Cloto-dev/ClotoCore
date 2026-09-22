# opverify-ledger

This branch holds the ledger written by the nightly opverify run
(`.github/workflows/opverify-nightly.yml` on the default branch).

The nightly reads its baseline from `qa/opverify/history.jsonl` on this branch
and appends one row per OS job here. It lives outside the default branch
because the default branch only accepts changes through a pull request, and a
ledger row is not a change anyone reviews.

Do not merge this branch into the default branch. Apex and local runs still
record to `qa/opverify/history.jsonl` on the default branch; each run is only
compared with earlier rows of the same target, so the two files never serve as
each other's baseline.
