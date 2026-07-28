# Governance

ProcHerd is maintained in public.

## Roles

- **Contributors** propose issues, fixtures, documentation, tests, code, and
  reviews.
- **Maintainers** triage reports, review changes, protect public contracts,
  manage security responses, and define releases.
- **Release managers** are maintainers authorized to create signed tags and
  trigger release automation.

The repository owner is the current maintainer and release manager. New
maintainers may be added after sustained constructive contributions and
demonstrated understanding of cross-platform process ownership, evidence
boundaries, and security limits.

## Decision process

Small reversible changes are decided through pull requests. Lifecycle/schema
changes, state migrations, new process controls, lease guarantees, dependency
policy, and security-boundary changes start with an issue and remain open for
feedback.

Decisions favor:

1. observable ownership and cleanup over optimistic state;
2. explicit loss, races, and capability differences over hidden guarantees;
3. bounded operations and conservative mutation;
4. stable machine-readable contracts;
5. evidence from failure fixtures, stress tests, and real workflows.

If consensus is not reached, a maintainer records the decision and rationale.
Security-sensitive details remain private until coordinated disclosure is safe.

## Changes and releases

Contributor pull requests need maintainer approval. Maintainer-authored pull
requests need a recorded self-review and all required checks. Required checks
are never bypassed for a release. Release requirements are in
[RELEASING.md](RELEASING.md); support windows are in [SECURITY.md](SECURITY.md).

## Project health

Maintainers periodically review dependency freshness, unanswered reports,
cross-platform failures, cleanup and lease stress results, schema
compatibility, release reproducibility, security reports, and opt-in adoption.
Governance will evolve with the contributor base.
