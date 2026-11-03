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
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().filter(|a| *a == "--json").count() > 1 {
        return Err(arguments());
    }
    args.retain(|a| a != "--json");
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
fn arguments() -> ContractError {
    ContractError::new(
        "INVALID_ARGUMENTS",
        "Use contract describe, contract schema, or validate --input <file|-> --structural; optional --json",
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
            (
                json!({"protocol_version":PROTOCOL_VERSION,"ok":false,"error":error}),
                2,
            )
        }
    };
    println!("{}", envelope);
    std::process::exit(exit);
}
