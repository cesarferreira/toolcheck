# Sample project

A self-contained development environment for trying toolcheck on macOS/Linux.
The executables in `bin/` are **fake tools that only print fixture versions**.
You do not need Ruby, dekit, robin, or openDHU installed. Upgrade guidance is
displayed only.

Install toolcheck first if needed:

```sh
cargo install toolcheck --locked
```

From the repository root:

```sh
cd examples/sample-project
TOOLCHECK="$(command -v toolcheck)"
PATH="$PWD/bin" "$TOOLCHECK" check
```

Resolve toolcheck before restricting PATH. The isolated PATH makes the sample
independent of real tool installations and avoids probing their versions.

| Tool | Installed | Policy | Expected result |
| --- | --- | --- | --- |
| dekit | `0.10.10` | Minimum `0.10.3` | Pass; numeric SemVer ordering |
| ruby | `3.3.0` | Existing `.ruby-version` pin | Pass; `exact_from` |
| robin | `1.4.2` | Minimum `1.4` | Pass; custom arguments and regex capture |
| opendhu (`aa`) | `0.9.0` | Minimum `1.2.3`, warning severity | Warn; version read from stderr |
| extra | Missing | Optional | Skipped |

The summary is **3 passed, 1 warning, 1 skipped**, with exit code `0`.
The robin fixture prints a separate build version, so the regex selects its
release version. The openDHU floor is illustrative.

Try JSON output and strict checks:

```sh
PATH="$PWD/bin" "$TOOLCHECK" check --json
PATH="$PWD/bin" "$TOOLCHECK" check --verbose # Full paths and explanations
PATH="$PWD/bin" "$TOOLCHECK" check --strict
# Strict mode exits 1 because of the openDHU warning.
```

Try automatic initialization without changing the committed policy:

```sh
PATH="$PWD/bin" "$TOOLCHECK" init --config starter.toml
```

This discovers `.ruby-version`, creates a Ruby `exact_from` entry, and adds
commented examples. `starter.toml` is ignored by Git. Running the same command
again refuses to overwrite the file and exits `2`.
