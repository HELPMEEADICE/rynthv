use super::*;
use clap::{ArgMatches, Command};
use rustyline::{
    Context, Editor, Helper,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::Highlighter,
    hint::Hinter,
    history::DefaultHistory,
    validate::Validator,
};
use serde::Deserialize;
use std::{io::BufRead, sync::atomic::Ordering};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: Value,
    args: Vec<String>,
    #[serde(default)]
    input: Option<Value>,
}

impl Workstation {
    pub(super) fn install_interrupt(&self) {
        let cancelled = Arc::clone(&self.cancellation);
        if let Err(error) = ctrlc::set_handler(move || cancelled.store(true, Ordering::Release)) {
            eprintln!("Could not install Ctrl-C handler: {error}");
        }
    }

    pub(super) fn stdio(&mut self) -> ExitCode {
        self.streaming = true;
        self.install_interrupt();
        let input = io::stdin();
        let mut input = input.lock();
        let output = io::stdout();
        let mut output = output.lock();
        loop {
            let mut line = Vec::new();
            let count = match (&mut input)
                .take(MAX_INPUT + 1)
                .read_until(b'\n', &mut line)
            {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
            };
            self.cancellation.store(false, Ordering::Release);
            let mut shutdown = false;
            let mut name = String::new();
            let mut id = Value::Null;
            let result = if count as u64 > MAX_INPUT {
                if line.last() != Some(&b'\n') && input.skip_until(b'\n').is_err() {
                    return ExitCode::FAILURE;
                }
                Err(invalid("JSON Lines request exceeds 64 MiB"))
            } else {
                serde_json::from_slice::<Request>(&line)
                    .map_err(|error| invalid(error.to_string()))
                    .and_then(|request| {
                        id = request.id;
                        if !id.is_string() && !id.is_i64() && !id.is_u64() {
                            return Err(invalid("Request id must be a string or integer"));
                        }
                        if request.args == ["exit"] {
                            name = "exit".into();
                            self.ensure_clean()?;
                            shutdown = true;
                            return Ok(json!({"closed":true}));
                        }
                        let parsed = self.parse_session(request.args)?;
                        let call =
                            Call::parse(&parsed).ok_or_else(|| invalid("Specify a command"))?;
                        name = call.name();
                        self.request_input = request.input;
                        let result = self.execute(call);
                        self.request_input = None;
                        result
                    })
            };
            let mut reply = envelope(&name, &result);
            reply["id"] = id;
            if serde_json::to_writer(&mut output, &reply).is_err()
                || writeln!(output).is_err()
                || output.flush().is_err()
            {
                return ExitCode::FAILURE;
            }
            if shutdown {
                return ExitCode::SUCCESS;
            }
        }
        if let Err(error) = self.ensure_clean() {
            eprintln!("{}: {}", error.code, error.message);
            return ExitCode::from(error.exit());
        }
        ExitCode::SUCCESS
    }

    pub(super) fn repl(&mut self) -> ExitCode {
        self.streaming = true;
        self.install_interrupt();
        let mut editor = match Editor::<Completion, DefaultHistory>::new() {
            Ok(editor) => editor,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        };
        editor.set_helper(Some(Completion));
        eprintln!("RynthV Headless Workstation\nType help for commands, or exit to close.");
        loop {
            let prompt = if self.open.as_ref().is_some_and(|open| open.editor.dirty()) {
                "rynthv*> "
            } else {
                "rynthv> "
            };
            let line = match editor.readline(prompt) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => continue,
                Err(ReadlineError::Eof) => {
                    if let Err(error) = self.ensure_clean() {
                        eprintln!("{}: {}", error.code, error.message);
                        // The terminal may have been closed, so EOF must not
                        // enter a retry loop.
                        return ExitCode::from(error.exit());
                    }
                    return ExitCode::SUCCESS;
                }
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let _ = editor.add_history_entry(line.as_str());
            let args = match shell_words::split(&line) {
                Ok(args) => args,
                Err(error) => {
                    eprintln!("{error}");
                    continue;
                }
            };
            if args == ["exit"] || args == ["quit"] {
                match self.ensure_clean() {
                    Ok(()) => return ExitCode::SUCCESS,
                    Err(error) => eprintln!("{}: {}", error.code, error.message),
                }
                continue;
            }
            self.cancellation.store(false, Ordering::Release);
            match self.parse_session(args) {
                Ok(parsed) => {
                    if let Some(call) = Call::parse(&parsed) {
                        let name = call.name();
                        let _ = emit(
                            &name,
                            self.execute(call),
                            call.flag("json"),
                            call.flag("quiet"),
                        );
                    }
                }
                Err(error) if error.code == "HELP" => print!("{}", error.message),
                Err(error) => eprintln!("{}: {}", error.code, error.message),
            }
        }
    }

    pub(super) fn parse_session(&self, mut args: Vec<String>) -> Result<ArgMatches> {
        let parse = |args: &[String]| {
            commands::command().try_get_matches_from(
                std::iter::once("rynthv -cli").chain(args.iter().map(String::as_str)),
            )
        };
        if args.len() >= 3
            && args[0] == "track"
            && args[1] == "add"
            && !args[2].starts_with('-')
            && !args[2].to_ascii_lowercase().ends_with(".svp")
            && !args
                .iter()
                .any(|arg| arg == "--name" || arg.starts_with("--name="))
            && self.open.is_some()
        {
            args.insert(2, "--name".into());
        }
        let first = parse(&args);
        let display = first.as_ref().err().is_some_and(|error| {
            matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            )
        });
        let missing_project = first
            .as_ref()
            .ok()
            .and_then(Call::parse)
            .is_some_and(|call| call.family == "render" && call.get("project").is_none());
        let parsed = if !display && (first.is_err() || missing_project) {
            if let Some(open) = &self.open
                && let Some((index, required)) = project_arguments(&args)
            {
                let mut scoped = args.clone();
                for (option, value) in [
                    ("--track", open.track.as_ref()),
                    ("--group", open.group.as_ref()),
                ] {
                    if let Some(value) = value
                        && required.contains(&option)
                        && !args.iter().any(|arg| {
                            arg == option
                                || arg
                                    .strip_prefix(option)
                                    .is_some_and(|value| value.starts_with('='))
                        })
                    {
                        scoped.extend([option.to_owned(), value.clone()]);
                    }
                }
                let scope_only = parse(&scoped);
                if scope_only
                    .as_ref()
                    .ok()
                    .and_then(Call::parse)
                    .is_some_and(|call| call.get("project").is_some())
                {
                    scope_only
                } else {
                    let path = open.path.to_string_lossy().into_owned();
                    args.insert(index, path.clone());
                    scoped.insert(index, path);
                    parse(&args).or_else(|_| parse(&scoped)).or(first)
                }
            } else {
                first
            }
        } else {
            first
        };
        parsed.map_err(|error| {
            let code = if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                "HELP"
            } else {
                "USAGE"
            };
            Failure::new(code, error.to_string())
        })
    }
}

fn project_arguments(args: &[String]) -> Option<(usize, Vec<&'static str>)> {
    let mut root = commands::command();
    root.build();
    let first = args
        .iter()
        .position(|word| root.find_subcommand(word).is_some())?;
    let family = root.find_subcommand(&args[first])?;
    let second = args
        .get(first + 1)
        .and_then(|name| family.find_subcommand(name));
    let (command, index) = second.map_or((family, first + 1), |command| (command, first + 2));
    if family.get_name() == "project" && matches!(command.get_name(), "new" | "open") {
        return None;
    }
    let has_project = command.get_arguments().any(|arg| arg.get_id() == "project");
    let required = ["--track", "--group"]
        .into_iter()
        .filter(|name| {
            command
                .get_arguments()
                .any(|arg| arg.is_required_set() && arg.get_long() == Some(&name[2..]))
        })
        .collect();
    has_project.then_some((index, required))
}

struct Completion;
impl Helper for Completion {}
impl Hinter for Completion {
    type Hint = String;
}
impl Highlighter for Completion {}
impl Validator for Completion {}
impl Completer for Completion {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        cursor: usize,
        _: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let text = &line[..cursor];
        let start = text.rfind(char::is_whitespace).map_or(0, |index| index + 1);
        let prefix = text[start..].trim_start();
        let words = shell_words::split(&text[..start]).unwrap_or_default();
        let mut root = commands::command();
        root.build();
        let mut selected: &Command = &root;
        for word in words.iter().filter(|word| !word.starts_with('-')) {
            if let Some(command) = selected.find_subcommand(word) {
                selected = command;
            }
        }
        let options = selected
            .get_arguments()
            .filter_map(|argument| argument.get_long())
            .map(|name| format!("--{name}"));
        let commands = selected
            .get_subcommands()
            .map(|command| command.get_name().to_owned());
        let candidates = options
            .chain(commands)
            .filter(|word| word.starts_with(prefix))
            .map(|word| Pair {
                display: word.clone(),
                replacement: format!("{word} "),
            })
            .collect();
        Ok((start, candidates))
    }
}
