<div align="center">
  <h1>toolcheck</h1>

  <p><strong>Check team tool versions and explain required upgrades</strong></p>

  <p>
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
    <img alt="Edition" src="https://img.shields.io/badge/edition-2024-blue">
    <a href="https://crates.io/crates/toolcheck"><img alt="crates.io" src="https://img.shields.io/crates/v/toolcheck.svg"></a>
  </p>

  <p>
    <a href="#install">Install</a>
    &nbsp;·&nbsp;
    <a href="#quickstart">Quickstart</a>
  </p>
</div>

---

## Install

Requires [Rust](https://rustup.rs) **1.85+** and `~/.cargo/bin` on your `PATH`.

```bash
cargo install toolcheck --locked
```

Verify:

```bash
toolcheck --help
```

<details>
<summary><strong>Build from source</strong> — for development or unreleased changes</summary>

```bash
git clone https://github.com/cesarferreira/toolcheck.git
cd toolcheck
cargo install --path . --locked
# or
make install-release
```

Debug install (faster compile, larger binary):

```bash
make install
```

Run without installing:

```bash
make build-release
./target/release/toolcheck
```

</details>

<a id="quickstart"></a>
## Quickstart

For a runnable example with bundled fake executables and predictable results,
see the [sample project](examples/sample-project/README.md). It demonstrates pins,
custom commands, warnings, and optional tools without installing those tools.

Create a starter policy in your project:

```bash
toolcheck init
# Or: toolcheck init --config config/team.toml --json
```

`init` discovers plain version pins beside the destination manifest:
`.ruby-version`, `.bazelversion`, `.node-version`, `.nvmrc`, `.python-version`,
and `.terraform-version`. It writes `exact_from` references to the existing files,
plus commented examples for arbitrary executables. It never runs version commands
or overwrites existing files, including symlinks. The destination directory must
already exist. Invalid, unreadable, or conflicting pins are reported for manual
review; formats such as `lts/*` are not inferred. When equivalent node pins exist,
`.node-version` takes precedence over `.nvmrc`.

If no usable pins are found, the policy contains commented examples only; edit it
before checking tools. `--json` reports detected tools and warnings. Then commit
your reviewed `toolcheck.toml`, for example:

```toml
schema_version = 1

[tools.dekit]
minimum = "0.10.3"
reason = "Required for Kitty graphics passthrough"
upgrade = "curl -fsSL https://dekit.run/install.sh | sh"

[tools.opendhu]
command = "aa"
minimum = "1.2.3" # Illustrative policy, not a verified openDHU floor
severity = "warn"
version_args = ["--version"]
upgrade = "Follow your team's openDHU installation guide"

[tools.ruby]
exact_from = ".ruby-version" # Reads your existing plain version pin
upgrade = "mise install"
```

```bash
toolcheck                          # Same as toolcheck check
toolcheck check --strict           # Warnings also fail CI
toolcheck check --verbose          # Include paths and full explanations
toolcheck --config config/team.toml --json
```

Any executable works, including private `robin` binaries and openDHU's `aa`.
There is no registry or tool-specific integration. Checks run offline, with
no installation, upgrades, network requests, shell hooks, or project writes by
toolcheck. `init` writes only the requested starter manifest. Version commands
run as ordinary subprocesses; their own behavior is outside toolcheck's control. Upgrade text is displayed and **never executed**.

### Policy

Unknown fields, invalid versions/regex/timeouts, contradictory bounds, and missing
or invalid pin files are configuration errors. The entire policy is validated
before any version command runs. An empty tools table is valid.

| Field | Meaning / default |
| --- | --- |
| `command` | Executable name (defaults to the logical tool name), or a path; no embedded arguments |
| `minimum`, `maximum` | Inclusive SemVer bounds; either or both may be set |
| `exact` | Exact version; cannot be combined with bounds or `exact_from` |
| `exact_from` | Relative path from the manifest directory to an existing UTF-8 plain version file; exclusive with other constraints |
| `version_args` | Literal argument array, default `["--version"]`; `[]` passes no arguments |
| `version_regex` | Optional Rust regex; capture group **1** must contain the version |
| `severity` | `"error"` (default) or `"warn"` for installed-tool violations and probe failures |
| `optional` | `false` by default; only missing executables are skipped |
| `reason`, `upgrade` | Explanatory text and display-only remediation |
| `timeout_ms` | Integer, 1–30000; default 2000; shared across selected and shadow version probes |

Set at least one constraint. Range expressions such as `^1.2`, tool registries,
and complex pin formats (`.tool-versions`, JSON, lockfiles) are not supported.
Pins contain just one version, e.g. `3.3.0` with an optional trailing newline.
Do not copy existing pins into `exact`; point to them with `exact_from`.

### Versions and PATH

Versions use SemVer numeric ordering: `0.10.10` meets `minimum = "0.10.3"`.
A leading lowercase `v` is removed; short numeric versions are padded (`2` →
`2.0.0`, `3.3` → `3.3.0`). Invalid SemVer, including leading zeroes, is rejected.
Build metadata does not affect comparisons. Prereleases are excluded unless a
constraint explicitly names a prerelease with the same major/minor/patch;
then SemVer precedence applies. For example `2.0.0-beta.2` meets a minimum of
`2.0.0-beta.1`, but not a minimum of `1.0.0` or `2.0.0`.

Both stdout and stderr are inspected. Automatic extraction recognizes whole,
whitespace-separated version tokens, preferring dotted versions. Multiple
different versions are ambiguous and fail; supply a regex for unusual output:

```toml
[tools.robin]
minimum = "1.4"
version_args = ["version"]
version_regex = 'release: (v[0-9]+\.[0-9]+\.[0-9]+)'
```

On macOS/Linux, resolution selects the first executable file on `PATH`, skipping
non-executable entries. Relative command paths are relative to the invocation's
working directory. No shell runs, so shell aliases/functions and shell command
caches do not apply. Selected paths and all distinct shadowed paths are reported;
symlinks and hard links to the same file count once. When the selected version is
known and incompatible, up to three shadowed installations are probed using the
remaining tool deadline. A compatible shadow produces PATH-order guidance.
Unprobed installations remain listed with `issue = "not_probed"`.

### Results and limits

Human output uses a compact table, with guidance for warnings and failures.
Colors are automatic in terminals; `NO_COLOR` disables automatic color.
Use `--color always` or `--color never` to override, and `--verbose` for paths,
reasons, and every shadowed installation. JSON output is unchanged by these flags.

| Exit | Meaning |
| --- | --- |
| `0` | Acceptable, including warnings unless `--strict` |
| `1` | Missing required executable, error result, or warning under `--strict` |
| `2` | Invalid configuration/invocation or report I/O failure |

Missing required executables always fail, even with `severity = "warn"`.
Installed optional tools are checked normally. Timeouts, nonzero exits,
unparseable/ambiguous versions, and output limits produce explicit issues and an
unknown actual version; they never pass silently.

`check --json` emits one JSON object on stdout with `schema_version`, tools sorted by
logical name, and a summary. Each tool has `status` (`pass`, `warn`, `error`,
`skipped`), `issue`, `actual`, `required`, `selected_path`, `shadowed`, `reason`,
`remediation`, and `message`. Unknown values are `null`; path bytes that cannot be
represented in UTF-8 use replacement characters. Configuration/invocation errors
instead have `error_kind` and `error`. JSON contains no progress or color output.

Each probe captures at most **64 KiB total** across stdout and stderr with bounded
reader buffers. Standard input is closed. Deadlines include waiting for child
processes holding output pipes open. Unix process groups are terminated after
probing; checks do not wait indefinitely for inherited pipes. Policy files are
limited to 1 MiB and pin files to 4 KiB. The MVP is verified on macOS; the hermetic
suite also targets Linux. Windows builds are unverified and only the direct child
can be terminated there; escaped/detached descendants are not guaranteed to stop.

## License

MIT
