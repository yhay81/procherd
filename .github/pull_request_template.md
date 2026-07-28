## User problem

<!-- What process-lifecycle or integration problem does this solve? -->

## Scope and guarantees

<!-- Include operating-system differences and limits of the guarantee. -->

## Verification

<!-- Exact commands, fixtures, platforms, negative cases, and repetition count. -->

## Release-safety checklist

- [ ] Commands remain argument vectors; no implicit shell parsing was added.
- [ ] Process-tree ownership and terminal supervisor shutdown stay observable.
- [ ] I tested failure, timeout, race, high-output, and cleanup paths when relevant.
- [ ] Logs remain bounded and loss is explicit; readiness cites retained evidence.
- [ ] Readiness remains local-only unless a reviewed contract explicitly changes it.
- [ ] Lease handoff, ownership, and deletion guarantees are documented honestly.
- [ ] I updated schemas, platform docs, examples, and the changelog for public changes.
- [ ] I ran formatting, clippy, tests, and package validation.
- [ ] I did not commit secrets, owner tokens, private paths, state, or logs.
- [ ] I documented compatibility or migration impact.
