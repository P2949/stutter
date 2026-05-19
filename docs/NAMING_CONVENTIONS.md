# Naming Conventions

Suffixes describe ownership boundaries:

- `FooArgs` is raw Clap input.
- `FooRequest` is raw HTTP or remote API input.
- `FooInput` is service-layer input after parsing.
- `FooConfigLayer` is a partial config source.
- `FooConfig` is resolved configuration.
- `FooRuntimeConfig` is execution-only configuration.
- `FooReport` is structured command or service output.
- `FooResponse` is an HTTP/API response DTO.

When a type crosses a boundary, prefer creating a new type with the boundary suffix over reusing the upstream type. For example, `RulesImportCommandInput` is the service input produced from the Clap-only `CliRulesImportArgs`.
