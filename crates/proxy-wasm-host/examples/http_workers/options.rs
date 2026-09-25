//! The command line of the example.

use std::fmt;
use std::path::PathBuf;

/// The command line the example accepts.
pub const USAGE: &str =
    "usage: http_workers [PATH | --wasm PATH | --no-wasm] [--workers N] [--port N]";

/// Which plugin the workers run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plugin {
    /// The plugin that ships with the example.
    Default,
    /// A plugin at a path you gave.
    Path(PathBuf),
    /// No plugin, so each worker answers the request as it arrived.
    None,
}

/// The options of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Which plugin the workers run.
    pub plugin: Plugin,
    /// How many worker threads serve requests.
    pub workers: usize,
    /// The port the server listens on.
    pub port: u16,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            plugin: Plugin::Default,
            workers: 4,
            port: 2045,
        }
    }
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Serve requests with these options.
    Run(Options),
    /// Print the usage line and stop.
    Help,
}

/// A command line the example cannot run with, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The options as they are read, before they are checked against each other.
#[derive(Default)]
struct Given {
    path: Option<PathBuf>,
    wasm: Option<PathBuf>,
    no_wasm: bool,
    workers: Option<usize>,
    port: Option<u16>,
}

/// Reads the arguments that follow the program name.
///
/// # Errors
///
/// Returns a [`UsageError`] for a value that is not valid, an option with no
/// value, an unknown option, an option given twice, and a plugin given in two
/// ways.
pub fn parse<I>(args: I) -> Result<Command, UsageError>
where
    I: IntoIterator<Item = String>,
{
    let mut given = Given::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help),
            "--no-wasm" if given.no_wasm => return Err(twice(&arg)),
            "--no-wasm" => given.no_wasm = true,
            "--wasm" => set(&mut given.wasm, &arg, plugin_path(value(&mut args, &arg)?)?)?,
            "--workers" => set(&mut given.workers, &arg, workers(&value(&mut args, &arg)?)?)?,
            "--port" => set(&mut given.port, &arg, port(&value(&mut args, &arg)?)?)?,
            flag if flag.starts_with('-') => {
                return Err(UsageError(format!("unknown option {flag}")));
            }
            _ if given.path.is_some() => {
                return Err(UsageError("give one plugin path".to_owned()));
            }
            _ => given.path = Some(plugin_path(arg)?),
        }
    }
    let plugin = match (given.path, given.wasm, given.no_wasm) {
        (None, None, false) => Plugin::Default,
        (Some(path), None, false) | (None, Some(path), false) => Plugin::Path(path),
        (None, None, true) => Plugin::None,
        (Some(_), Some(_), _) => {
            return Err(UsageError(
                "give the plugin as a path or with --wasm, not both".to_owned(),
            ));
        }
        (_, _, true) => {
            return Err(UsageError("--no-wasm takes no plugin".to_owned()));
        }
    };
    let defaults = Options::default();
    Ok(Command::Run(Options {
        plugin,
        workers: given.workers.unwrap_or(defaults.workers),
        port: given.port.unwrap_or(defaults.port),
    }))
}

fn twice(flag: &str) -> UsageError {
    UsageError(format!("{flag} is given twice"))
}

fn set<T>(slot: &mut Option<T>, flag: &str, value: T) -> Result<(), UsageError> {
    if slot.is_some() {
        return Err(twice(flag));
    }
    *slot = Some(value);
    Ok(())
}

/// The value after `flag`, which is missing when the next argument is itself
/// an option.
fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, UsageError> {
    match args.next() {
        Some(value) if !value.starts_with("--") => Ok(value),
        _ => Err(UsageError(format!("{flag} needs a value"))),
    }
}

fn plugin_path(value: String) -> Result<PathBuf, UsageError> {
    if value.is_empty() {
        return Err(UsageError("the plugin path is empty".to_owned()));
    }
    Ok(PathBuf::from(value))
}

fn workers(value: &str) -> Result<usize, UsageError> {
    let digits = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    match value.parse::<usize>() {
        Ok(count) if digits && count > 0 => Ok(count),
        _ => Err(UsageError(format!(
            "--workers needs a whole number of 1 or more, not {value}"
        ))),
    }
}

fn port(value: &str) -> Result<u16, UsageError> {
    let digits = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    match value.parse::<u16>() {
        Ok(port) if digits && port > 0 => Ok(port),
        _ => Err(UsageError(format!(
            "--port needs a whole number from 1 to 65535, not {value}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn usage(message: &str) -> Result<Command, UsageError> {
        Err(UsageError(message.to_owned()))
    }

    #[test]
    fn a_worker_count_that_is_not_a_number_is_a_usage_error() {
        // Arrange
        let line = args("--workers abc");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(
            result,
            usage("--workers needs a whole number of 1 or more, not abc")
        );
    }

    #[test]
    fn a_worker_count_of_zero_is_a_usage_error() {
        // Arrange
        let line = args("--workers 0");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(
            result,
            usage("--workers needs a whole number of 1 or more, not 0")
        );
    }

    #[test]
    fn a_port_out_of_range_is_a_usage_error() {
        // Arrange
        let lines = [args("--port 0"), args("--port 65536")];

        // Act
        let results = lines.map(parse);

        // Assert
        assert_eq!(
            results,
            [
                usage("--port needs a whole number from 1 to 65535, not 0"),
                usage("--port needs a whole number from 1 to 65535, not 65536"),
            ]
        );
    }

    #[test]
    fn no_wasm_with_a_plugin_is_a_usage_error() {
        // Arrange
        let lines = [
            args("--no-wasm some.wasm"),
            args("--no-wasm --wasm some.wasm"),
        ];

        // Act
        let results = lines.map(parse);

        // Assert
        assert_eq!(
            results,
            [
                usage("--no-wasm takes no plugin"),
                usage("--no-wasm takes no plugin"),
            ]
        );
    }

    #[test]
    fn both_plugin_forms_are_a_usage_error() {
        // Arrange
        let line = args("a.wasm --wasm b.wasm");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(
            result,
            usage("give the plugin as a path or with --wasm, not both")
        );
    }

    #[test]
    fn an_option_with_no_value_is_a_usage_error() {
        // Arrange
        let line = args("--workers");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(result, usage("--workers needs a value"));
    }

    #[test]
    fn an_option_given_twice_is_a_usage_error() {
        // Arrange
        let lines = [args("--workers 1 --workers 2"), args("--no-wasm --no-wasm")];

        // Act
        let results = lines.map(parse);

        // Assert
        assert_eq!(
            results,
            [
                usage("--workers is given twice"),
                usage("--no-wasm is given twice")
            ]
        );
    }

    #[test]
    fn an_unknown_option_and_a_second_path_are_usage_errors() {
        // Arrange
        let lines = [args("--threads 8"), args("a.wasm b.wasm")];

        // Act
        let results = lines.map(parse);

        // Assert
        assert_eq!(
            results,
            [
                usage("unknown option --threads"),
                usage("give one plugin path")
            ]
        );
    }

    #[test]
    fn no_arguments_give_the_defaults() {
        // Arrange
        let line = args("");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(
            result,
            Ok(Command::Run(Options {
                plugin: Plugin::Default,
                workers: 4,
                port: 2045,
            }))
        );
    }

    #[test]
    fn each_option_reads_back() {
        // Arrange
        let line = args("--wasm a.wasm --workers 1 --port 8080");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(
            result,
            Ok(Command::Run(Options {
                plugin: Plugin::Path(PathBuf::from("a.wasm")),
                workers: 1,
                port: 8080,
            }))
        );
    }

    #[test]
    fn a_bare_path_is_the_plugin_and_no_wasm_is_no_plugin() {
        // Arrange
        let lines = [args("a.wasm"), args("--no-wasm --workers 2")];

        // Act
        let results = lines.map(parse);

        // Assert
        assert_eq!(
            results,
            [
                Ok(Command::Run(Options {
                    plugin: Plugin::Path(PathBuf::from("a.wasm")),
                    ..Options::default()
                })),
                Ok(Command::Run(Options {
                    plugin: Plugin::None,
                    workers: 2,
                    ..Options::default()
                })),
            ]
        );
    }

    #[test]
    fn an_option_where_its_value_should_be_is_a_usage_error() {
        // Arrange
        let lines = [args("--wasm --no-wasm"), args("--workers --port 8080")];

        // Act
        let results = lines.map(parse);

        // Assert
        assert_eq!(
            results,
            [
                usage("--wasm needs a value"),
                usage("--workers needs a value")
            ]
        );
    }

    #[test]
    fn an_empty_plugin_path_is_a_usage_error() {
        // Arrange
        let lines = [
            vec![String::new()],
            vec!["--wasm".to_owned(), String::new()],
        ];

        // Act
        let results = lines.map(parse);

        // Assert
        assert_eq!(
            results,
            [
                usage("the plugin path is empty"),
                usage("the plugin path is empty")
            ]
        );
    }

    #[test]
    fn a_worker_count_with_a_sign_is_a_usage_error() {
        // Arrange
        let line = args("--workers +2");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(
            result,
            usage("--workers needs a whole number of 1 or more, not +2")
        );
    }

    #[test]
    fn help_asks_for_the_usage_line() {
        // Arrange
        let line = args("--workers 2 --help");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(result, Ok(Command::Help));
    }
}
