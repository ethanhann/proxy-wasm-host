//! The command line of the example.

use std::fmt;
use std::path::PathBuf;

use proxy_wasm_host::OptLevel;

/// The command line the example accepts.
pub const USAGE: &str = "usage: http_workers [PATH | --wasm PATH | --no-wasm] [--workers N] \
                         [--port N] [--opt-level speed|speed-and-size]";

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
    /// How hard the compiler optimizes the plugin.
    pub opt_level: OptLevel,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            plugin: Plugin::Default,
            workers: 4,
            port: 2045,
            opt_level: OptLevel::Speed,
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
    opt_level: Option<OptLevel>,
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
            "--wasm" => set(
                &mut given.wasm,
                &arg,
                PathBuf::from(value(&mut args, &arg)?),
            )?,
            "--workers" => set(&mut given.workers, &arg, workers(&value(&mut args, &arg)?)?)?,
            "--port" => set(&mut given.port, &arg, port(&value(&mut args, &arg)?)?)?,
            "--opt-level" => set(
                &mut given.opt_level,
                &arg,
                opt_level(&value(&mut args, &arg)?)?,
            )?,
            flag if flag.starts_with('-') => {
                return Err(UsageError(format!("unknown option {flag}")));
            }
            _ if given.path.is_some() => {
                return Err(UsageError("give one plugin path".to_owned()));
            }
            _ => given.path = Some(PathBuf::from(arg)),
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
        opt_level: given.opt_level.unwrap_or(defaults.opt_level),
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

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, UsageError> {
    args.next()
        .ok_or_else(|| UsageError(format!("{flag} needs a value")))
}

fn workers(value: &str) -> Result<usize, UsageError> {
    match value.parse::<usize>() {
        Ok(count) if count > 0 => Ok(count),
        _ => Err(UsageError(format!(
            "--workers needs a whole number of 1 or more, not {value}"
        ))),
    }
}

fn port(value: &str) -> Result<u16, UsageError> {
    match value.parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(UsageError(format!(
            "--port needs a whole number from 1 to 65535, not {value}"
        ))),
    }
}

fn opt_level(value: &str) -> Result<OptLevel, UsageError> {
    match value {
        "speed" => Ok(OptLevel::Speed),
        "speed-and-size" => Ok(OptLevel::SpeedAndSize),
        _ => Err(UsageError(format!(
            "--opt-level needs speed or speed-and-size, not {value}"
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
    fn an_unknown_opt_level_is_a_usage_error() {
        // Arrange
        let line = args("--opt-level fast");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(
            result,
            usage("--opt-level needs speed or speed-and-size, not fast")
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
                opt_level: OptLevel::Speed,
            }))
        );
    }

    #[test]
    fn each_option_reads_back() {
        // Arrange
        let line = args("--wasm a.wasm --workers 1 --port 8080 --opt-level speed-and-size");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(
            result,
            Ok(Command::Run(Options {
                plugin: Plugin::Path(PathBuf::from("a.wasm")),
                workers: 1,
                port: 8080,
                opt_level: OptLevel::SpeedAndSize,
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
    fn help_asks_for_the_usage_line() {
        // Arrange
        let line = args("--workers 2 --help");

        // Act
        let result = parse(line);

        // Assert
        assert_eq!(result, Ok(Command::Help));
    }
}
