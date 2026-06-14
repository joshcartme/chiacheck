# chiacheck

A frontend health score calculator.

`chiacheck` measures the health of a frontend project by running configurable metrics
(linting, test coverage, type errors, AST checks, custom scripts) and accumulating a
**penalty score** — `0` is perfect, higher means more issues. It can also score a range of
git commits and generate an HTML trend report.

## Install

```bash
npm install -g chiacheck
# or run without installing:
npx chiacheck score
```

This package is a thin launcher. On install, npm pulls in exactly one prebuilt binary for
your platform via `optionalDependencies` (`@chiacheck/<platform>`); the launcher then execs it.
No compilation, no postinstall download.

## Usage

```bash
chiacheck score
chiacheck range --from <SHA> --to <SHA> --output report.html
chiacheck history --days 30 --output history.html
```

See the [project README](https://github.com/joshcartme/chiacheck#readme) for full configuration
and metric documentation.

## Supported platforms

| OS      | Arch  | Package              |
|---------|-------|----------------------|
| macOS   | arm64 | `@chiacheck/darwin-arm64`|
| macOS   | x64   | `@chiacheck/darwin-x64`  |
| Linux   | x64   | `@chiacheck/linux-x64`   |
| Linux   | arm64 | `@chiacheck/linux-arm64` |
| Windows | x64   | `@chiacheck/win32-x64`   |

Linux packages target glibc (musl/Alpine is not yet supported).

## License

MIT
