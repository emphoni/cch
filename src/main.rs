use chrono::Local;
use clap::{Parser, Subcommand};
use rusqlite::{Connection, params};
use serde::Serialize;
use std::env;
use std::os::unix::process::CommandExt; // Unix-only: exec() replaces the process
use std::path::PathBuf;
use std::process::Command;
use tiny_http::{Header, Method, Response, Server};

const DASHBOARD_HTML: &str = include_str!("../dashboard.html");

fn db_path() -> PathBuf {
    let mut p = home_dir().join(".cch");
    std::fs::create_dir_all(&p).ok();
    p.push("sessions.db");
    p
}

fn home_dir() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .expect("$HOME is not set")
}

fn get_db() -> Connection {
    let conn = Connection::open(db_path()).expect("Failed to open database");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            pwd TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .expect("Failed to create table");
    conn
}

#[derive(Serialize)]
struct Session {
    id: String,
    title: String,
    pwd: String,
    created_at: String,
}

fn save_session(session_id: &str, title: &str) {
    let pwd = env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let now = Local::now().format("%Y-%m-%dT%H:%M:%S%.6f").to_string();
    let db = get_db();
    db.execute(
        "INSERT OR REPLACE INTO sessions (id, title, pwd, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![session_id, title, pwd, now],
    )
    .expect("Failed to save session");
    println!("Saved: {title}");
    println!("  ID:  {session_id}");
    println!("  Dir: {pwd}");
}

fn list_sessions(limit: usize) {
    let db = get_db();
    let mut stmt = db
        .prepare("SELECT id, title, pwd, created_at FROM sessions ORDER BY created_at DESC LIMIT ?1")
        .unwrap();
    let rows: Vec<Session> = stmt
        .query_map(params![limit], |row| {
            Ok(Session {
                id: row.get(0)?,
                title: row.get(1)?,
                pwd: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .unwrap()
        .map(|r| r.expect("failed to read session row"))
        .collect();

    if rows.is_empty() {
        println!("No saved sessions.");
        return;
    }

    for (i, s) in rows.iter().enumerate() {
        let ts = &s.created_at[..std::cmp::min(16, s.created_at.len())].replace('T', " ");
        println!("[{}] {}", i + 1, s.title);
        println!("    ID:  {}", s.id);
        println!(
            "    Cmd: claude --resume {} --dangerously-skip-permissions",
            s.id
        );
        println!("    Dir: {}  ({ts})", s.pwd);
        if i < rows.len() - 1 {
            println!();
        }
    }
}

fn search_sessions(query: &str) {
    let db = get_db();
    let pattern = format!("%{query}%");
    let mut stmt = db
        .prepare("SELECT id, title, pwd, created_at FROM sessions WHERE title LIKE ?1 OR id LIKE ?1 ORDER BY created_at DESC")
        .unwrap();
    let rows: Vec<Session> = stmt
        .query_map(params![pattern], |row| {
            Ok(Session {
                id: row.get(0)?,
                title: row.get(1)?,
                pwd: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .unwrap()
        .map(|r| r.expect("failed to read session row"))
        .collect();

    if rows.is_empty() {
        println!("No sessions matching '{query}'.");
        return;
    }

    for (i, s) in rows.iter().enumerate() {
        let ts = &s.created_at[..std::cmp::min(16, s.created_at.len())].replace('T', " ");
        println!("[{}] {}", i + 1, s.title);
        println!("    ID:  {}", s.id);
        println!(
            "    Cmd: claude --resume {} --dangerously-skip-permissions",
            s.id
        );
        println!("    Dir: {}  ({ts})", s.pwd);
        if i < rows.len() - 1 {
            println!();
        }
    }
}

fn get_all_sessions(db: &Connection) -> Vec<Session> {
    let mut stmt = db
        .prepare("SELECT id, title, pwd, created_at FROM sessions ORDER BY created_at DESC")
        .unwrap();
    stmt.query_map([], |row| {
        Ok(Session {
            id: row.get(0)?,
            title: row.get(1)?,
            pwd: row.get(2)?,
            created_at: row.get(3)?,
        })
    })
    .unwrap()
    .map(|r| r.expect("failed to read session row"))
    .collect()
}

fn resume_session(identifier: &str) {
    let db = get_db();

    if let Ok(idx) = identifier.parse::<usize>() {
        let rows = get_all_sessions(&db);
        if idx >= 1 && idx <= rows.len() {
            let s = &rows[idx - 1];
            do_resume(&s.id, &s.pwd, &s.title);
            return;
        }
        println!("Index {identifier} out of range. Use `cch ls` to see sessions.");
        return;
    }

    // Exact match
    let session: Option<Session> = db
        .query_row(
            "SELECT id, title, pwd, created_at FROM sessions WHERE id = ?1",
            params![identifier],
            |row| {
                Ok(Session {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    pwd: row.get(2)?,
                    created_at: row.get(3)?,
                })
            },
        )
        .ok();

    // Partial match fallback
    let session = session.or_else(|| {
        let pattern = format!("%{identifier}%");
        db.query_row(
            "SELECT id, title, pwd, created_at FROM sessions WHERE id LIKE ?1",
            params![pattern],
            |row| {
                Ok(Session {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    pwd: row.get(2)?,
                    created_at: row.get(3)?,
                })
            },
        )
        .ok()
    });

    match session {
        Some(s) => do_resume(&s.id, &s.pwd, &s.title),
        None => println!("No session found for '{identifier}'."),
    }
}

fn do_resume(session_id: &str, pwd: &str, title: &str) {
    println!("Resuming: {title}");
    println!("  Dir: {pwd}");
    println!("  Cmd: claude --resume {session_id}");
    env::set_current_dir(pwd).ok();
    let err = Command::new("claude")
        .args(["--resume", session_id])
        .exec();
    eprintln!("Failed to exec claude: {err}");
}

fn delete_session(identifier: &str) {
    let db = get_db();

    if let Ok(idx) = identifier.parse::<usize>() {
        let rows = get_all_sessions(&db);
        if idx >= 1 && idx <= rows.len() {
            let s = &rows[idx - 1];
            db.execute("DELETE FROM sessions WHERE id = ?1", params![s.id])
                .unwrap();
            let short = &s.id[..std::cmp::min(8, s.id.len())];
            println!("Deleted: {} ({short}...)", s.title);
            return;
        }
        println!("Index {identifier} out of range.");
        return;
    }

    let deleted = db
        .execute("DELETE FROM sessions WHERE id = ?1", params![identifier])
        .expect("failed to delete session");
    if deleted > 0 {
        println!("Deleted {deleted} session(s).");
        return;
    }
    let pattern = format!("%{identifier}%");
    let deleted = db
        .execute("DELETE FROM sessions WHERE id LIKE ?1", params![pattern])
        .expect("failed to delete session");
    if deleted > 0 {
        println!("Deleted {deleted} session(s).");
    } else {
        println!("No session found for '{identifier}'.");
    }
}

fn start_web(port: u16) {
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).expect("Failed to start server");
    let url = format!("http://localhost:{port}");
    println!("cch dashboard \u{2192} {url}");
    println!("Press Ctrl+C to stop");
    open::that(&url).ok();

    for request in server.incoming_requests() {
        match (request.method(), request.url()) {
            (&Method::Get, "/") => {
                let response = Response::from_string(DASHBOARD_HTML)
                    .with_header(Header::from_bytes("Content-Type", "text/html").unwrap());
                request.respond(response).ok();
            }
            (&Method::Get, "/api/sessions") => {
                let db = get_db();
                let rows = get_all_sessions(&db);
                let json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string());
                let response = Response::from_string(json)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                request.respond(response).ok();
            }
            (&Method::Delete, url) if url.starts_with("/api/sessions/") => {
                let session_id = &url["/api/sessions/".len()..];
                let db = get_db();
                db.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
                    .ok();
                let response = Response::from_string(r#"{"ok":true}"#)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                request.respond(response).ok();
            }
            _ => {
                let response = Response::from_string("Not Found").with_status_code(404);
                request.respond(response).ok();
            }
        }
    }
}

#[derive(Parser)]
#[command(name = "cch", about = "Claude Code Helper — manage session contexts")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Save a session
    #[command(alias = "s")]
    Save {
        session_id: String,
        title: String,
    },
    /// List saved sessions
    #[command(alias = "list")]
    Ls {
        #[arg(short, default_value = "20")]
        n: usize,
    },
    /// Search sessions by title or ID
    #[command(alias = "f")]
    Find {
        query: String,
    },
    /// Resume a session
    #[command(alias = "r")]
    Resume {
        identifier: String,
    },
    /// Delete a saved session
    #[command(alias = "del")]
    Rm {
        identifier: String,
    },
    /// Open the web dashboard
    #[command(alias = "w")]
    Web {
        #[arg(short, long, default_value = "5111")]
        port: u16,
    },
}

fn main() {
    // Handle shorthand: cch <id> "title" (when first arg isn't a known subcommand)
    let raw_args: Vec<String> = env::args().collect();
    if raw_args.len() >= 3 {
        let known = [
            "save", "s", "ls", "list", "find", "f", "resume", "r", "rm", "del", "web", "w",
            "-h", "--help", "help",
        ];
        if !known.contains(&raw_args[1].as_str()) {
            save_session(&raw_args[1], &raw_args[2]);
            return;
        }
    }

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Save { session_id, title }) => save_session(&session_id, &title),
        Some(Commands::Ls { n }) => list_sessions(n),
        Some(Commands::Find { query }) => search_sessions(&query),
        Some(Commands::Resume { identifier }) => resume_session(&identifier),
        Some(Commands::Rm { identifier }) => delete_session(&identifier),
        Some(Commands::Web { port }) => start_web(port),
        None => {
            Cli::parse_from(["cch", "--help"]);
        }
    }
}
