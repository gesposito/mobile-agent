//! `batch`, as agent-browser runs it: commands as quoted strings, or a JSON
//! array of argv arrays on standard input, run in order on the client, each
//! one request to the daemon. `--bail` stops at the first failure. In JSON
//! mode the output is one array of `{command, success, result, error}` plus
//! `code` and `warning` when set; otherwise each command's output in turn,
//! a blank line between them. The exit status is 1 when any command failed.

use std::io::Read;
use std::path::Path;

use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches};
use serde_json::{Map, Value};

use crate::args::{BatchArgs, Cli, Command};
use crate::client::{Failure, Reply};
use crate::code::{INVALID_ARGUMENT, UNKNOWN_COMMAND};
use crate::{Settings, execute, output, write};

/// Splits a command line into words as a POSIX shell does: whitespace
/// separates words, single quotes keep everything up to the next single
/// quote, double quotes keep everything but a backslash before `"`, `\`, `$`
/// or a backtick, and a backslash outside quotes keeps the next character.
pub fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err(format!("unclosed single quote in {line:?}")),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => word.push(c),
                            Some(c) => {
                                word.push('\\');
                                word.push(c);
                            }
                            None => return Err(format!("unclosed double quote in {line:?}")),
                        },
                        Some(c) => word.push(c),
                        None => return Err(format!("unclosed double quote in {line:?}")),
                    }
                }
            }
            '\\' => {
                started = true;
                match chars.next() {
                    Some(c) => word.push(c),
                    None => return Err(format!("a trailing backslash in {line:?}")),
                }
            }
            c => {
                started = true;
                word.push(c);
            }
        }
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

/// Parses one batch entry. The batch's settings apply, except a session,
/// device, platform or force flag written on the entry's own command line.
fn parse(argv: &[String], batch: &Settings) -> Result<(Command, Settings), Failure> {
    let mut line = vec!["mobile-agent".to_owned()];
    line.extend(argv.iter().cloned());
    let parsed = Cli::command()
        .try_get_matches_from(&line)
        .and_then(|matches| Cli::from_arg_matches(&matches).map(|cli| (cli, matches)));
    let (cli, matches) = parsed.map_err(|error| {
        let code = if error.kind() == clap::error::ErrorKind::InvalidSubcommand {
            UNKNOWN_COMMAND
        } else {
            INVALID_ARGUMENT
        };
        let message = error.to_string();
        Failure::new(code, message.trim().trim_start_matches("error: "))
    })?;
    if matches!(cli.command, Command::Batch(_)) {
        return Err(Failure::new(
            INVALID_ARGUMENT,
            "batch cannot run inside batch",
        ));
    }
    let own = |id: &str| matches.value_source(id) == Some(ValueSource::CommandLine);
    let mut settings = batch.clone();
    if own("session")
        && let Some(session) = cli.global.session.clone()
    {
        settings.session = session;
    }
    if own("device") {
        settings.device = cli.global.device.clone();
    }
    if own("platform") {
        settings.platform = cli.global.platform.clone();
    }
    if own("force") {
        settings.force = cli.global.force;
    }
    Ok((cli.command, settings))
}

/// One command of a batch.
struct Entry {
    argv: Vec<String>,
    /// Why the command line could not be split into words.
    problem: Option<String>,
}

/// The entries to run: the arguments' commands split into words, else a
/// JSON array of argv arrays read from standard input.
fn entries(args: &BatchArgs) -> Result<Vec<Entry>, Failure> {
    if !args.commands.is_empty() {
        return Ok(args
            .commands
            .iter()
            .map(|line| match split(line) {
                Ok(argv) => Entry {
                    argv,
                    problem: None,
                },
                Err(problem) => Entry {
                    argv: vec![line.clone()],
                    problem: Some(problem),
                },
            })
            .collect());
    }
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| Failure::new(INVALID_ARGUMENT, format!("cannot read standard input: {e}")))?;
    let commands: Vec<Vec<String>> = serde_json::from_str(&input).map_err(|e| {
        Failure::new(
            INVALID_ARGUMENT,
            format!(
                "invalid JSON input: {e}; expected an array of string arrays, such as [[\"open\", \"dev.example.app\"], [\"snapshot\", \"-i\"]]"
            ),
        )
    })?;
    Ok(commands
        .into_iter()
        .map(|argv| Entry {
            argv,
            problem: None,
        })
        .collect())
}

/// One entry of the JSON array.
fn result_entry(argv: &[String], reply: &Reply) -> Value {
    let mut entry = Map::new();
    entry.insert("command".into(), argv.to_vec().into());
    entry.insert("success".into(), reply.success.into());
    entry.insert("result".into(), reply.data.clone());
    entry.insert("error".into(), reply.error.clone().into());
    if let Some(code) = &reply.code {
        entry.insert("code".into(), code.clone().into());
    }
    if let Some(warning) = &reply.warning {
        entry.insert("warning".into(), warning.clone().into());
    }
    Value::Object(entry)
}

/// Runs a batch and returns the exit status.
pub fn run(args: &BatchArgs, settings: &Settings, cwd: &Path) -> i32 {
    let entries = match entries(args) {
        Ok(entries) => entries,
        Err(failure) => return crate::finish(settings.json, "", &Reply::failed(failure)),
    };
    let mut results = Vec::new();
    let mut failed = false;
    let mut printed = false;
    for Entry { argv, problem } in entries {
        if argv.is_empty() {
            continue;
        }
        let (action, reply) = match problem {
            Some(problem) => (
                String::new(),
                Reply::failed(Failure::new(INVALID_ARGUMENT, problem)),
            ),
            None => match parse(&argv, settings) {
                Ok((command, entry_settings)) => execute(&command, &entry_settings, cwd),
                Err(failure) => (String::new(), Reply::failed(failure)),
            },
        };
        failed |= !reply.success;
        if settings.json {
            results.push(result_entry(&argv, &reply));
        } else {
            let (out, err) = output::human(&action, &reply);
            if !out.is_empty() {
                if printed {
                    write(&mut std::io::stdout().lock(), "\n");
                }
                write(&mut std::io::stdout().lock(), &out);
                printed = true;
            }
            write(&mut std::io::stderr().lock(), &err);
        }
        if !reply.success && args.bail {
            break;
        }
    }
    if settings.json {
        write(
            &mut std::io::stdout().lock(),
            &format!("{}\n", Value::Array(results)),
        );
    }
    i32::from(failed)
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn words_split_as_a_shell_splits_them() {
        assert_eq!(split("click @e2").unwrap(), ["click", "@e2"]);
        assert_eq!(
            split(r#"fill #name "agent smith""#).unwrap(),
            ["fill", "#name", "agent smith"]
        );
        assert_eq!(
            split("get text '#login-status'").unwrap(),
            ["get", "text", "#login-status"]
        );
        assert_eq!(
            split(r#"type "say \"hi\"" x\ y"#).unwrap(),
            ["type", "say \"hi\"", "x y"]
        );
        assert_eq!(split("fill x ''").unwrap(), ["fill", "x", ""]);
        assert_eq!(split("  ").unwrap(), Vec::<String>::new());
        assert!(split("fill 'x").is_err());
        assert!(split("fill \"x").is_err());
    }
}
