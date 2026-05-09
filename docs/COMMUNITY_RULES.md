# Community rules

`stutter` can use community-maintained process-name rules as classification hints
when its local process-tree scanner leaves a task as `Unknown`.

The core `stutter` package does **not** ship the full GPL Ananicy rules database.
The built-in asset under `stutter/assets/community-rules/` is only a tiny test
fixture for schema and classifier tests.

## Licensing boundary

The intended split is:

```text
Core stutter:
  ✅ importer code
  ✅ stutter rules import
  ✅ user-data storage
  ✅ runtime loader
  ❌ no full GPL database embedded

Separate GPL repo/package:
  ✅ optional convenience package with pre-generated rule files
```

This keeps the core project free to keep its existing license status while still
letting users opt into GPL community data locally.

## Import a local Ananicy-compatible checkout

Users who want Ananicy-compatible rules should clone or otherwise obtain the
rules themselves, then import that local checkout:

```bash
git clone https://github.com/CachyOS/ananicy-rules /tmp/ananicy-rules

stutter rules import \
  --source /tmp/ananicy-rules \
  --source-repo https://github.com/CachyOS/ananicy-rules \
  --source-commit "$(git -C /tmp/ananicy-rules rev-parse HEAD)"
```

Check the installed state with:

```bash
stutter rules status
```

List imported rule files with:

```bash
stutter rules list
```

Remove an imported rule set with:

```bash
stutter rules remove --name ananicy
```

Use `--dry-run` with `rules import` or `rules remove` to inspect what would
happen without writing or deleting files.

## Storage layout

Generated/imported rule databases are user data, not hand-written config, so
they are stored under XDG data directories.

When `XDG_DATA_HOME` is set:

```text
$XDG_DATA_HOME/stutter/community-rules/ananicy.generated.json
$XDG_DATA_HOME/stutter/community-rules/ananicy.metadata.json
```

When `XDG_DATA_HOME` is not set:

```text
~/.local/share/stutter/community-rules/ananicy.generated.json
~/.local/share/stutter/community-rules/ananicy.metadata.json
```

The hand-written config file remains separate:

```text
~/.config/stutter/config.toml
```

Do not put generated community-rule JSON in `~/.config/stutter`.

## Config file

Community rules can be controlled from the normal user config file:

```toml
[community_rules]
enabled = true
sources = ["user"]
```

Explicit generated rule files can also be listed:

```toml
[community_rules]
enabled = true
paths = ["/path/to/custom.generated.json"]
```

Runtime loading priority is:

```text
explicit paths > user data > system data > test fixture only in tests
```

Normal release builds should not use the test fixture as real classification
data.

## System and package-provided rules

User-driven imports are the default path, but distributions may provide optional
pre-generated community-rule packages.

Package-provided generated files should use the same system data layout that the
runtime loader already searches:

```text
/usr/share/stutter/community-rules/ananicy.generated.json
/usr/share/stutter/community-rules/ananicy.metadata.json
```

Local administrator overrides may also be installed under:

```text
/usr/local/share/stutter/community-rules/ananicy.generated.json
/usr/local/share/stutter/community-rules/ananicy.metadata.json
```

The generated rule file must be the reduced `stutter` community-rules schema. The
metadata file should identify the source project, license, source repository,
source commit, generation timestamp, and generated rule filename.

A clean Gentoo split would be:

```text
app-admin/stutter
  MIT/Apache core binary
  importer code
  runtime loader
  no full GPL Ananicy database embedded

app-admin/stutter-community-rules-ananicy
  GPL package
  optional runtime companion for app-admin/stutter
  installs /usr/share/stutter/community-rules/ananicy.generated.json
  installs /usr/share/stutter/community-rules/ananicy.metadata.json
```

This gives users a one-command install path while preserving the licensing
boundary: core `stutter` does not embed or vendor the full GPL rules database,
and the optional community-rules package carries the GPL data and its license
metadata separately.

## Classification hints only

Community rules are classification hints. They are used to turn otherwise
unknown tasks into task classes such as `Game` when the process identity and
context are convincing enough.

Imported community rules must not copy Ananicy scheduling policy into `stutter`.
The importer must discard scheduling fields such as:

```text
nice values
ionice values
scheduler class
CPU affinity
systemd policy
```

`stutter` decides its own scheduling, affinity, profiling, and recommendation
behavior. Community rules only help identify what a process likely is.

## Ambiguous executable names

Generic names such as these are ambiguous:

```text
build.exe
launcher.exe
game.exe
setup.exe
client.exe
server.exe
```

Rules for ambiguous names require useful context such as Steam, Proton, Wine, or
compatdata paths before classification should happen. This avoids treating
ordinary build tools or helper processes as game tasks just because their
basename appears in a community rule database.
