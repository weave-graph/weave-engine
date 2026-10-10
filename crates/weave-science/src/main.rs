use std::{
    env,
    io::{self, Read},
    process,
};
use weave_engine::{Error, HostContext};
use weave_science::{parse_request, Response, ScienceSession, MAX_INPUT_BYTES};

fn input_error(message: impl Into<String>) -> Error {
    Error {
        code: "E_INPUT".into(),
        message: message.into(),
    }
}

fn run() -> Result<Response, Error> {
    let mut args = env::args().skip(1);
    let mut db = None;
    let mut actor = None;
    let mut graphs = vec![];
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| input_error(format!("missing value for {flag}")))?;
        match flag.as_str() {
            "--db" if db.is_none() => db = Some(value),
            "--actor" if actor.is_none() => actor = Some(value),
            "--write" => graphs.push(value),
            _ => return Err(input_error(format!("unknown or repeated flag: {flag}"))),
        }
    }
    let db = db.ok_or_else(|| input_error("--db required"))?;
    let actor = actor
        .filter(|value| !value.is_empty())
        .ok_or_else(|| input_error("nonempty --actor required"))?;
    let mut bytes = vec![];
    io::stdin()
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| input_error(error.to_string()))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(input_error("request exceeds 16 MiB"));
    }
    let request = parse_request(&bytes)?;
    // Parse before opening storage so malformed input cannot create a database.
    let mut session = ScienceSession::open(db, HostContext::new(actor, graphs))?;
    Ok(session.respond(request))
}

fn main() {
    if env::args()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!("weave-science --db PATH --actor PRINCIPAL [--write GRAPH ...]\nReads one JSON request from stdin and writes one JSON response to stdout.\nOperations: capabilities, import, execute, analyze.");
        return;
    }
    let response = match run() {
        Ok(response) => response,
        Err(error) => Response::error(error),
    };
    let succeeded = response.ok;
    match serde_json::to_string(&response) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("response serialization failed: {error}");
            process::exit(1);
        }
    }
    if !succeeded {
        process::exit(1);
    }
}
