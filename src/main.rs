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
const FAVICON: &[u8] = include_bytes!("../favicon.ico");
const MARKED_JS: &str = include_str!("../vendor/marked.min.js");

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

#[derive(Serialize)]
struct SessionDetails {
    id: String,
    title: String,
    pwd: String,
    created_at: String,
    messages: Vec<ChatMessage>,
    stats: SessionStats,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    text: String,
    timestamp: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<String>,
}

#[derive(Serialize)]
struct SessionStats {
    user_messages: usize,
    assistant_messages: usize,
    tool_calls: std::collections::HashMap<String, usize>,
    total_input_tokens: u64,
    total_output_tokens: u64,
    duration_minutes: u64,
}

fn find_session_jsonl(session_id: &str) -> Option<PathBuf> {
    let claude_dir = home_dir().join(".claude").join("projects");
    if !claude_dir.exists() {
        return None;
    }
    for entry in std::fs::read_dir(&claude_dir).ok()? {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir() {
            let jsonl = entry.path().join(format!("{session_id}.jsonl"));
            if jsonl.exists() {
                return Some(jsonl);
            }
        }
    }
    None
}

fn parse_session_details(session_id: &str) -> Option<SessionDetails> {
    let jsonl_path = find_session_jsonl(session_id)?;
    let content = std::fs::read_to_string(&jsonl_path).ok()?;

    let mut messages = Vec::new();
    let mut tool_calls = std::collections::HashMap::new();
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut first_ts: Option<String> = None;
    let mut last_ts: Option<String> = None;

    for line in content.lines() {
        let d: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = d.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if msg_type != "user" && msg_type != "assistant" {
            continue;
        }

        let ts = d.get("timestamp").and_then(|t| t.as_str()).unwrap_or("").to_string();
        if !ts.is_empty() {
            if first_ts.is_none() {
                first_ts = Some(ts.clone());
            }
            last_ts = Some(ts.clone());
        }

        let msg = match d.get("message") {
            Some(m) => m,
            None => continue,
        };

        if msg_type == "user" {
            if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                if !text.trim().is_empty() {
                    messages.push(ChatMessage {
                        role: "user".into(),
                        text: text.into(),
                        timestamp: ts,
                        tools: vec![],
                    });
                }
            }
        } else if msg_type == "assistant" {
            if let Some(usage) = msg.get("usage") {
                total_input += usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                total_output += usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
            }

            if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                let mut texts = Vec::new();
                let mut tools = Vec::new();
                for c in content_arr {
                    match c.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = c.get("text").and_then(|t| t.as_str()) {
                                if !t.trim().is_empty() {
                                    texts.push(t.to_string());
                                }
                            }
                        }
                        Some("tool_use") => {
                            if let Some(name) = c.get("name").and_then(|n| n.as_str()) {
                                tools.push(name.to_string());
                                *tool_calls.entry(name.to_string()).or_insert(0) += 1;
                            }
                        }
                        _ => {}
                    }
                }
                if !texts.is_empty() || !tools.is_empty() {
                    messages.push(ChatMessage {
                        role: "assistant".into(),
                        text: texts.join("\n"),
                        timestamp: ts,
                        tools,
                    });
                }
            }
        }
    }

    // Duration
    let duration = match (&first_ts, &last_ts) {
        (Some(t1), Some(t2)) => {
            let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();
            match (parse(t1), parse(t2)) {
                (Some(a), Some(b)) => (b - a).num_minutes().unsigned_abs(),
                _ => 0,
            }
        }
        _ => 0,
    };

    let user_count = messages.iter().filter(|m| m.role == "user").count();
    let assistant_count = messages.iter().filter(|m| m.role == "assistant").count();

    // DB metadata
    let db = get_db();
    let row: Option<Session> = db
        .query_row(
            "SELECT id, title, pwd, created_at FROM sessions WHERE id = ?1",
            params![session_id],
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

    Some(SessionDetails {
        id: session_id.to_string(),
        title: row.as_ref().map(|r| r.title.clone()).unwrap_or_else(|| session_id[..8.min(session_id.len())].to_string()),
        pwd: row.as_ref().map(|r| r.pwd.clone()).unwrap_or_default(),
        created_at: row.as_ref().map(|r| r.created_at.clone()).unwrap_or_else(|| first_ts.unwrap_or_default()),
        messages,
        stats: SessionStats {
            user_messages: user_count,
            assistant_messages: assistant_count,
            tool_calls,
            total_input_tokens: total_input,
            total_output_tokens: total_output,
            duration_minutes: duration,
        },
    })
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

fn rename_session(identifier: &str, new_title: &str) {
    let db = get_db();

    if let Ok(idx) = identifier.parse::<usize>() {
        let rows = get_all_sessions(&db);
        if idx >= 1 && idx <= rows.len() {
            let s = &rows[idx - 1];
            db.execute("UPDATE sessions SET title = ?1 WHERE id = ?2", params![new_title, s.id])
                .expect("failed to rename session");
            println!("Renamed: {} -> {new_title}", s.title);
            return;
        }
        println!("Index {identifier} out of range.");
        return;
    }

    let updated = db
        .execute("UPDATE sessions SET title = ?1 WHERE id = ?2", params![new_title, identifier])
        .expect("failed to rename session");
    if updated > 0 {
        println!("Renamed to: {new_title}");
        return;
    }
    let pattern = format!("%{identifier}%");
    let updated = db
        .execute("UPDATE sessions SET title = ?1 WHERE id LIKE ?2", params![new_title, pattern])
        .expect("failed to rename session");
    if updated > 0 {
        println!("Renamed to: {new_title}");
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

    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();
        match (method, url.as_str()) {
            (Method::Get, "/favicon.ico") => {
                let response = Response::from_data(FAVICON.to_vec())
                    .with_header(Header::from_bytes("Content-Type", "image/x-icon").unwrap());
                request.respond(response).ok();
            }
            (Method::Get, "/vendor/marked.min.js") => {
                let response = Response::from_string(MARKED_JS)
                    .with_header(Header::from_bytes("Content-Type", "application/javascript").unwrap());
                request.respond(response).ok();
            }
            (Method::Get, "/api/db") => {
                let data = std::fs::read(db_path()).unwrap_or_default();
                let response = Response::from_data(data)
                    .with_header(Header::from_bytes("Content-Type", "application/octet-stream").unwrap())
                    .with_header(Header::from_bytes("Content-Disposition", "attachment; filename=\"sessions.db\"").unwrap());
                request.respond(response).ok();
            }
            (Method::Get, url) if url.starts_with("/api/sessions/") && url.ends_with("/details") => {
                let session_id = &url["/api/sessions/".len()..url.len() - "/details".len()];
                match parse_session_details(session_id) {
                    Some(details) => {
                        let json = serde_json::to_string(&details).unwrap_or_else(|_| "{}".to_string());
                        let response = Response::from_string(json)
                            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                        request.respond(response).ok();
                    }
                    None => {
                        let response = Response::from_string(r#"{"error":"session not found"}"#)
                            .with_status_code(404)
                            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                        request.respond(response).ok();
                    }
                }
            }
            (Method::Get, "/api/sessions") => {
                let db = get_db();
                let rows = get_all_sessions(&db);
                let json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string());
                let response = Response::from_string(json)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                request.respond(response).ok();
            }
            (Method::Put, url) if url.starts_with("/api/sessions/") => {
                let session_id = &url["/api/sessions/".len()..];
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).ok();
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(title) = val.get("title").and_then(|t| t.as_str()) {
                        let db = get_db();
                        db.execute("UPDATE sessions SET title = ?1 WHERE id = ?2", params![title, session_id]).ok();
                    }
                }
                let response = Response::from_string(r#"{"ok":true}"#)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                request.respond(response).ok();
            }
            (Method::Delete, url) if url.starts_with("/api/sessions/") => {
                let session_id = &url["/api/sessions/".len()..];
                let db = get_db();
                db.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])
                    .ok();
                let response = Response::from_string(r#"{"ok":true}"#)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                request.respond(response).ok();
            }
            (Method::Get, _) => {
                // SPA fallback: serve dashboard for any unmatched GET
                let response = Response::from_string(DASHBOARD_HTML)
                    .with_header(Header::from_bytes("Content-Type", "text/html").unwrap());
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
    /// Rename a saved session
    Rename {
        identifier: String,
        title: String,
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
            "save", "s", "ls", "list", "find", "f", "resume", "r", "rename", "rm", "del", "web", "w",
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
        Some(Commands::Rename { identifier, title }) => rename_session(&identifier, &title),
        Some(Commands::Rm { identifier }) => delete_session(&identifier),
        Some(Commands::Web { port }) => start_web(port),
        None => {
            Cli::parse_from(["cch", "--help"]);
        }
    }
}
