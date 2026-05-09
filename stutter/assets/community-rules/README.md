# Community Rule Fixtures

`ananicy.fixture.generated.json` is a tiny test fixture for
`stutter::community_rules`.

This file is intentionally not a real bundled Ananicy database. It exists only to
exercise the reduced community-rules JSON schema and the runtime safety gates in
unit tests and documentation examples.

Do not place the full upstream `CachyOS/ananicy-rules` database in this asset
directory. Users who want Ananicy-compatible community rules should import their
own local checkout with:

```bash
stutter rules import --source /path/to/ananicy-rules
```

Imported rules are user-installed data and should be written under the XDG data
layout, for example:

```text
$XDG_DATA_HOME/stutter/community-rules/ananicy.generated.json
```

or, when `XDG_DATA_HOME` is unset:

```text
~/.local/share/stutter/community-rules/ananicy.generated.json
```

Generated or imported rule databases must contain only reduced identity hints
such as process names, normalized names, source paths, broad categories, and safe
context hints. They must not preserve Ananicy scheduling policy data such as nice
values, ionice values, scheduler classes, CPU affinity, or systemd policy.
