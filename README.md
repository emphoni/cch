# cch

Save and manage [Claude Code](https://docs.anthropic.com/en/docs/claude-code) session contexts. Zero dependencies, pure Python.

## Install

```bash
# Clone and alias
git clone https://github.com/emphoni/cch.git
echo 'alias cch="python3 /path/to/cch/cch.py"' >> ~/.zshrc
source ~/.zshrc
```

## Usage

```bash
# Save a session (from the directory you were working in)
cch f69ff62b-261e-432e-902a-239185645137 "Refactoring auth module"

# List sessions
cch ls

# Search
cch find auth

# Resume (by index, full ID, or partial ID)
cch resume 1
cch resume f69ff

# Delete
cch rm 1

# Web dashboard
cch web
```

## Web Dashboard

`cch web` opens a local dashboard at `localhost:5111` with sidebar navigation grouped by directory, search, copy-to-clipboard resume commands, and dark/light mode.

## Storage

SQLite database at `~/.cch/sessions.db`.

## License

MIT
