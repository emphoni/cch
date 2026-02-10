#!/usr/bin/env python3
"""cch - Claude Code Helper: Save and manage Claude Code session contexts."""

import argparse
import json
import os
import sqlite3
import sys
import webbrowser
from datetime import datetime
from functools import partial
from http.server import HTTPServer, BaseHTTPRequestHandler
from pathlib import Path

DB_PATH = Path.home() / ".cch" / "sessions.db"


def get_db():
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(DB_PATH))
    conn.row_factory = sqlite3.Row
    conn.execute(
        """CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            pwd TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"""
    )
    conn.commit()
    return conn


def save_session(session_id: str, title: str):
    pwd = os.getcwd()
    db = get_db()
    db.execute(
        "INSERT OR REPLACE INTO sessions (id, title, pwd, created_at) VALUES (?, ?, ?, ?)",
        (session_id, title, pwd, datetime.now().isoformat()),
    )
    db.commit()
    db.close()
    print(f"Saved: {title}")
    print(f"  ID:  {session_id}")
    print(f"  Dir: {pwd}")


def list_sessions(limit: int = 20):
    db = get_db()
    rows = db.execute(
        "SELECT * FROM sessions ORDER BY created_at DESC LIMIT ?", (limit,)
    ).fetchall()
    db.close()

    if not rows:
        print("No saved sessions.")
        return

    for i, row in enumerate(rows):
        ts = row["created_at"][:16].replace("T", " ")
        print(f"[{i+1}] {row['title']}")
        print(f"    ID:  {row['id']}")
        print(f"    Cmd: claude --resume {row['id']} --dangerously-skip-permissions")
        print(f"    Dir: {row['pwd']}  ({ts})")
        if i < len(rows) - 1:
            print()


def search_sessions(query: str):
    db = get_db()
    rows = db.execute(
        "SELECT * FROM sessions WHERE title LIKE ? OR id LIKE ? ORDER BY created_at DESC",
        (f"%{query}%", f"%{query}%"),
    ).fetchall()
    db.close()

    if not rows:
        print(f"No sessions matching '{query}'.")
        return

    for i, row in enumerate(rows):
        ts = row["created_at"][:16].replace("T", " ")
        print(f"[{i+1}] {row['title']}")
        print(f"    ID:  {row['id']}")
        print(f"    Cmd: claude --resume {row['id']} --dangerously-skip-permissions")
        print(f"    Dir: {row['pwd']}  ({ts})")
        if i < len(rows) - 1:
            print()


def resume_session(identifier: str):
    """Resume by session ID or by list index (e.g. '1' for most recent)."""
    db = get_db()

    # Try as a list index first
    if identifier.isdigit():
        idx = int(identifier) - 1
        rows = db.execute(
            "SELECT * FROM sessions ORDER BY created_at DESC"
        ).fetchall()
        db.close()
        if 0 <= idx < len(rows):
            row = rows[idx]
            _do_resume(row["id"], row["pwd"], row["title"])
            return
        print(f"Index {identifier} out of range. Use `cch ls` to see sessions.")
        return

    # Try as session ID (exact or partial match)
    row = db.execute("SELECT * FROM sessions WHERE id = ?", (identifier,)).fetchone()
    if not row:
        row = db.execute(
            "SELECT * FROM sessions WHERE id LIKE ?", (f"%{identifier}%",)
        ).fetchone()
    db.close()

    if row:
        _do_resume(row["id"], row["pwd"], row["title"])
    else:
        print(f"No session found for '{identifier}'.")


def _do_resume(session_id: str, pwd: str, title: str):
    cmd = f"claude --resume {session_id}"
    print(f"Resuming: {title}")
    print(f"  Dir: {pwd}")
    print(f"  Cmd: {cmd}")
    os.chdir(pwd)
    os.execlp("claude", "claude", "--resume", session_id)


def delete_session(identifier: str):
    db = get_db()
    if identifier.isdigit():
        idx = int(identifier) - 1
        rows = db.execute(
            "SELECT * FROM sessions ORDER BY created_at DESC"
        ).fetchall()
        if 0 <= idx < len(rows):
            row = rows[idx]
            db.execute("DELETE FROM sessions WHERE id = ?", (row["id"],))
            db.commit()
            db.close()
            print(f"Deleted: {row['title']} ({row['id'][:8]}...)")
            return
        db.close()
        print(f"Index {identifier} out of range.")
        return

    deleted = db.execute("DELETE FROM sessions WHERE id = ?", (identifier,)).rowcount
    if not deleted:
        deleted = db.execute(
            "DELETE FROM sessions WHERE id LIKE ?", (f"%{identifier}%",)
        ).rowcount
    db.commit()
    db.close()
    print(f"Deleted {deleted} session(s)." if deleted else f"No session found for '{identifier}'.")


DASHBOARD_PATH = Path(__file__).parent / "dashboard.html"


class CCHHandler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass  # silence request logs

    def do_GET(self):
        if self.path == "/api/sessions":
            db = get_db()
            rows = db.execute("SELECT * FROM sessions ORDER BY created_at DESC").fetchall()
            db.close()
            data = [dict(r) for r in rows]
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(data).encode())
        elif self.path == "/":
            html = DASHBOARD_PATH.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(html)
        else:
            self.send_error(404)

    def do_DELETE(self):
        if self.path.startswith("/api/sessions/"):
            session_id = self.path[len("/api/sessions/"):]
            db = get_db()
            db.execute("DELETE FROM sessions WHERE id = ?", (session_id,))
            db.commit()
            db.close()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"ok":true}')
        else:
            self.send_error(404)


def start_web(port: int = 5111):
    server = HTTPServer(("127.0.0.1", port), CCHHandler)
    url = f"http://localhost:{port}"
    print(f"cch dashboard → {url}")
    print("Press Ctrl+C to stop")
    webbrowser.open(url)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nStopped.")
        server.server_close()


def main():
    parser = argparse.ArgumentParser(
        prog="cch",
        description="Claude Code Helper - manage session contexts",
    )
    sub = parser.add_subparsers(dest="command")

    # cch save <id> "title"
    p_save = sub.add_parser("save", aliases=["s"], help="Save a session")
    p_save.add_argument("session_id", help="Claude Code session ID")
    p_save.add_argument("title", help="A descriptive title for the session")

    # cch ls
    p_ls = sub.add_parser("ls", aliases=["list"], help="List saved sessions")
    p_ls.add_argument("-n", type=int, default=20, help="Number of sessions to show")

    # cch find <query>
    p_find = sub.add_parser("find", aliases=["f"], help="Search sessions by title or ID")
    p_find.add_argument("query", help="Search term")

    # cch resume <id or index>
    p_resume = sub.add_parser("resume", aliases=["r"], help="Resume a session (by ID, partial ID, or list index)")
    p_resume.add_argument("identifier", help="Session ID, partial ID, or list index from `cch ls`")

    # cch rm <id or index>
    p_rm = sub.add_parser("rm", aliases=["del"], help="Delete a saved session")
    p_rm.add_argument("identifier", help="Session ID, partial ID, or list index")

    # cch web
    p_web = sub.add_parser("web", aliases=["w"], help="Open the web dashboard")
    p_web.add_argument("-p", "--port", type=int, default=5111, help="Port (default: 5111)")

    # If first arg isn't a known subcommand, treat as: cch <id> "title"
    known = ("save", "s", "ls", "list", "find", "f", "resume", "r", "rm", "del", "web", "w", "-h", "--help")
    if len(sys.argv) >= 3 and sys.argv[1] not in known:
        save_session(sys.argv[1], sys.argv[2])
        return

    args = parser.parse_args()

    if args.command in ("save", "s"):
        save_session(args.session_id, args.title)
    elif args.command in ("ls", "list"):
        list_sessions(args.n)
    elif args.command in ("find", "f"):
        search_sessions(args.query)
    elif args.command in ("resume", "r"):
        resume_session(args.identifier)
    elif args.command in ("rm", "del"):
        delete_session(args.identifier)
    elif args.command in ("web", "w"):
        start_web(args.port)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
