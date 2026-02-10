# cch

Save and manage [Claude Code](https://docs.anthropic.com/en/docs/claude-code) session contexts.

Two flavours — same commands, same database, pick what suits you:

| | Python | Rust |
|---|---|---|
| **Deps** | Python 3.9+ | None (single binary) |
| **Install** | Alias the script | Build or download binary |
| **Auditability** | Read `cch.py` directly | Read `src/main.rs` |
| **Binary size** | — | ~3 MB |

## Quick Start

```bash
curl -fsSL https://raw.githubusercontent.com/emphoni/cch/main/install.sh | sh
```

## Install

**Python** — zero build step, readable source:
```bash
git clone https://github.com/emphoni/cch.git ~/.cch-src && echo 'alias cch="python3 ~/.cch-src/cch.py"' >> ~/.zshrc && source ~/.zshrc
```

**Rust** — single binary, no runtime deps (requires [Rust toolchain](https://rustup.rs)):
```bash
git clone https://github.com/emphoni/cch.git && cd cch && cargo build --release && cp target/release/cch /usr/local/bin/
```

**Prebuilt binary** — no Rust needed:
Download from [Releases](https://github.com/emphoni/cch/releases) and drop in your `$PATH`.

Both share the same SQLite database at `~/.cch/sessions.db` — switch freely.

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

`cch web` opens a local dashboard at `localhost:5111` — sidebar grouped by directory, search, copy-to-clipboard resume commands, dark/light mode.

## License

MIT
