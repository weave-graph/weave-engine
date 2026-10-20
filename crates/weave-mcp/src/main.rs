use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process;
use weave_engine::HostContext;
use weave_mcp::{oversized_message, Server, MAX_MESSAGE_BYTES};
use weave_science::ScienceSession;

const USAGE: &str = "weave-mcp --db PATH --actor PRINCIPAL\n\
Read-only MCP preview over one existing local Weave store, on stdio.\n\
--actor is the reader identity every tool call runs as; per-record readers apply.\n\
There are no write tools and no authentication: local, single-user use only.";

/// The longest accepted principal, in bytes.
const MAX_ACTOR_BYTES: usize = 256;

fn configure() -> Result<(String, String), String> {
    let mut args = std::env::args().skip(1);
    let (mut db, mut actor) = (None, None);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--db" if db.is_none() => db = Some(value),
            "--actor" if actor.is_none() => actor = Some(value),
            _ => return Err(format!("unknown or repeated flag: {flag}")),
        }
    }
    let db = db.ok_or("--db is required")?;
    // There is no default identity: the host must name the reader.
    let actor = actor.ok_or("--actor is required")?;
    if actor.trim().is_empty()
        || actor.len() > MAX_ACTOR_BYTES
        || actor.chars().any(char::is_control)
        || actor.starts_with("weave:")
    {
        return Err(format!(
            "--actor must be a nonempty principal of at most {MAX_ACTOR_BYTES} bytes, without control characters, outside the reserved weave: namespace"
        ));
    }
    // A read-only preview never creates a store.
    if db != ":memory:" && !Path::new(&db).is_file() {
        return Err(format!("no store at {db}"));
    }
    Ok((db, actor))
}

enum Line {
    End,
    /// Longer than `MAX_MESSAGE_BYTES`; the line was consumed and dropped.
    Oversized,
    Message,
}

/// Read one line of at most `MAX_MESSAGE_BYTES` into `line`, without ever
/// buffering more than that.
fn read_line(input: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<Line> {
    line.clear();
    let mut oversized = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Ok(if oversized {
                Line::Oversized
            } else if line.is_empty() {
                Line::End
            } else {
                Line::Message
            });
        }
        let (chunk, done) = match buffer.iter().position(|b| *b == b'\n') {
            Some(i) => (&buffer[..i], i + 1),
            None => (buffer, buffer.len()),
        };
        let newline = done > chunk.len();
        if !oversized {
            if line.len() + chunk.len() > MAX_MESSAGE_BYTES {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        input.consume(done);
        if newline {
            return Ok(if oversized {
                Line::Oversized
            } else {
                Line::Message
            });
        }
    }
}

fn main() {
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!("{USAGE}");
        return;
    }
    let (db, actor) = match configure() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("weave-mcp: {message}\n{USAGE}");
            process::exit(2);
        }
    };
    // No write grants: even a write command would fail with E_FORBIDDEN.
    let host = HostContext::new(actor, Vec::<String>::new());
    let session = match ScienceSession::open(&db, host) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("weave-mcp: cannot open store: {}", error.code);
            process::exit(1);
        }
    };
    let mut server = Server::new(session);
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout().lock();
    let mut line = Vec::new();
    loop {
        let response = match read_line(&mut input, &mut line) {
            Ok(Line::End) => break,
            Ok(Line::Oversized) => Some(oversized_message()),
            Ok(Line::Message) => {
                let trimmed = line.trim_ascii();
                if trimmed.is_empty() {
                    continue;
                }
                server.handle_message(trimmed)
            }
            Err(error) => {
                eprintln!("weave-mcp: stdin: {error}");
                process::exit(1);
            }
        };
        if let Some(response) = response {
            let written = serde_json::to_writer(&mut output, &response)
                .map_err(io::Error::other)
                .and_then(|()| output.write_all(b"\n"))
                .and_then(|()| output.flush());
            if written.is_err() {
                process::exit(1);
            }
        }
    }
}
