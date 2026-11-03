use bodh::{ContractError, MAX_INPUT_BYTES, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::io::Read;

fn read_input(path: &str) -> Result<Vec<u8>, ContractError> {
    let reader: Box<dyn Read> =
        if path == "-" {
            Box::new(std::io::stdin())
        } else {
            Box::new(std::fs::File::open(path).map_err(|_| {
                ContractError::new("INPUT_UNREADABLE", "Cannot open input file", "")
            })?)
        };
    let mut input = Vec::new();
    reader
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut input)
        .map_err(|_| ContractError::new("INPUT_UNREADABLE", "Cannot read input", ""))?;
    Ok(input)
}
fn run() -> Result<Value, ContractError> {
    let mut args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().map_err(|_| arguments()))
        .collect::<Result<_, _>>()?;
    if args.iter().filter(|a| *a == "--json").count() > 1 {
        return Err(arguments());
    }
    args.retain(|a| a != "--json");
    if args.first().is_some_and(|a| a == "validate") && args.iter().any(|a| a == "--evidence-only")
    {
        return evidence_validation(&args[1..]);
    }
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["contract", "describe"] | ["--help"] => Ok(bodh::describe()),
        ["contract", "schema"] => Ok(bodh::schema()),
        ["validate", "--input", path, "--structural"]
        | ["validate", "--structural", "--input", path] => {
            bodh::validate_structure(&read_input(path)?)?;
            Ok(
                json!({"valid":true,"validation":"structural_only","semantic_validation_available":false}),
            )
        }
        _ => Err(arguments()),
    }
}
fn evidence_validation(args: &[String]) -> Result<Value, ContractError> {
    let mut options = std::collections::BTreeMap::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--evidence-only" {
            if options.insert(arg.as_str(), "").is_some() {
                return Err(arguments());
            }
            continue;
        }
        if !matches!(
            arg.as_str(),
            "--input" | "--context" | "--repository" | "--timeout-ms"
        ) {
            return Err(arguments());
        }
        let value = iter.next().ok_or_else(arguments)?;
        if options.insert(arg, value).is_some() {
            return Err(arguments());
        }
    }
    let input = options.get("--input").ok_or_else(arguments)?;
    let context = options.get("--context").ok_or_else(arguments)?;
    if *input == "-" && *context == "-" {
        return Err(arguments());
    }
    let repository = options.get("--repository").ok_or_else(arguments)?;
    let timeout = options
        .get("--timeout-ms")
        .map(|s| s.parse::<u64>().map_err(|_| arguments()))
        .transpose()?
        .unwrap_or(30_000);
    if timeout > 120_000 {
        return Err(arguments());
    }
    let prepared = bodh::parse_prepared_evidence(&read_input(context)?)?;
    let control = bodh::OperationControl::with_timeout(std::time::Duration::from_millis(timeout));
    bodh::validate_evidence(
        &read_input(input)?,
        &prepared,
        std::path::Path::new(repository),
        &control,
    )?;
    Ok(
        json!({"valid":true,"validation":"evidence_only","semantic_validation_complete":false,"remaining_checks":["findings_and_anchors","diagrams_and_follow_up"]}),
    )
}
fn exit_category(error: &ContractError) -> i32 {
    match error.code.as_str() {
        "CONTEXT_MISMATCH" => 3,
        "GIT_UNAVAILABLE" | "GIT_OBJECT_UNAVAILABLE" => 4,
        "OPERATION_TIMEOUT" | "CANCELLED" => 5,
        "GIT_FAILED" => 10,
        _ => 2,
    }
}
fn arguments() -> ContractError {
    ContractError::new(
        "INVALID_ARGUMENTS",
        "Use contract describe/schema, validate --input <file|-> --structural, or validate --input <file|-> --evidence-only --context <fixture.json> --repository <checkout> [--timeout-ms <0..120000>]; optional --json",
        "",
    )
}
fn main() {
    let (envelope, exit) = match run() {
        Ok(data) => (
            json!({"protocol_version":PROTOCOL_VERSION,"ok":true,"data":data}),
            0,
        ),
        Err(error) => {
            eprintln!("{}: {}", error.code, error.message);
            let exit = exit_category(&error);
            (
                json!({"protocol_version":PROTOCOL_VERSION,"ok":false,"error":error}),
                exit,
            )
        }
    };
    println!("{}", envelope);
    std::process::exit(exit);
}
