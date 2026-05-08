# Community Rule Fixtures

`ananicy.generated.json` is the reduced, auditable rule format consumed by
`stutter::community_rules`.

This initial file is intentionally tiny. It exercises the importer-shaped schema
and the runtime safety gates without vendoring the full upstream
`CachyOS/ananicy-rules` repository. A future generated refresh should record the
exact upstream commit and keep only reduced identity hints, not Ananicy
scheduling policy.
