"""opverify — ClotoCore operation-to-success verification harness.

A standing, OS-agnostic verification flow that drives a live ``clotocore``
kernel daemon over its HTTP admin API and asserts that a broad catalog of
real operations actually *succeed to completion* (not merely "does not
crash"). The harness itself only needs an HTTP endpoint + admin key; where
the daemon runs (local process, Linux VM, Windows VM) is a deployment
detail handled under ``opverify.deploy``.

Design authority: the approved operation-to-success plan. This package
is intentionally dependency-free (Python standard library only) so it runs
unchanged on a bare CI runner or inside a freshly-provisioned VM.
"""

__all__ = ["client", "oracle", "harness", "coverage"]
