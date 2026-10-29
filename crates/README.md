# Crate boundaries

The application composes these libraries. A shared data type should not require
opening the cache, loading application configuration, or starting a runtime.

| Crate | Owns |
| --- | --- |
| `kache-core` | Local store options and build-timeline records. |
| `kache-format` | Cache-entry metadata and format validation. |
| `kache-fs` | Filesystem operations, identities, and allocation measurements. |
| `kache-store` | Local artifact storage and its database. |
| `kache-shims` | Installed-binary selection and compiler-name links. |
| `kache-protocol` | Shared local-daemon request payloads and reporting records. |
| `kache-ipc` | Local endpoint naming, transport, listener ownership, and peer checks. |

`kache-protocol` depends on `kache-core` and has no storage or runtime
dependency. JSON compatibility defaults belong here and must continue to
describe the messages older clients and daemons sent.

`kache-ipc` provides synchronous clients, daemon listeners, and peer checks. It
delegates socket ownership to `kunobi-daemon`. Process startup, readiness, and
replacement also use `kunobi-daemon`, through the application's existing
lifecycle adapters.

Application policy stays outside these crates: configuration resolution,
selection of cache directories, and formatting of CLI output. Adapters turn
runtime state into protocol records.

## Remaining dependencies

The root package still builds one `kache` executable plus the embedded
build-script launcher. This extraction does not change process routing,
installation, signatures, or which process performs cache work.

The complete request and response envelopes live in `kache-protocol`, including
publication events, prediction rows, and aggregate storage reports. Store memo
inputs convert at the application boundary; storage access and validation remain
with the application. JSON names, omitted fields, and compatibility defaults
remain unchanged.

Compiler invocation code also reads and modifies process-wide environment and
uses global state. Moving it into a concurrent server requires explicit
per-request inputs and ownership of cancellation, descriptors, and output.
Extracting crates alone does not make that code safe to run concurrently.

## Validation

`make check` tests the full workspace and checks the client crates separately so
feature unification cannot hide a dependency in the client-only configuration.

Protocol tests cover older messages and prevent clients from supplying internal
prefetch attribution. IPC tests cover endpoint limits, reachability, and peer
admission. Changes to execution or cache policy need their own behavioral tests
and performance measurements.
