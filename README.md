# cch

Save and manage [Claude Code](https://docs.anthropic.com/en/docs/claude-code) session contexts.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/emphoni/cch/main/install.sh | sh
```

Or download a prebuilt binary from [Releases](https://github.com/emphoni/cch/releases) and drop it in your `$PATH`.

## Usage

```bash
# Save (shorthand)
cch f69ff62b-261e-432e-902a-239185645137 "Refactoring auth module"

# List
cch ls

# Search
cch find auth

# Resume (by index, ID, or partial ID)
cch resume 1
cch resume f69ff

# Delete
cch rm 1

# Web dashboard
cch web
```

## Web Dashboard

`cch web` opens a local dashboard at `localhost:5111` - sidebar grouped by directory, search, copy-to-clipboard resume commands, dark/light mode.

## License

MIT
