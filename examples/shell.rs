//! This is small, simple shell. Right now it can't handle pipelines, and
//! doesn't yet look up executables in the `PATH`. It has one built-in: `cd`. It
//! illustrates how some UNIX functions like `fork` and `execve` can be used
//! idiomatically in Rust with `nix`, and how `std` and `nix` can be used
//! together.

#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate scopeguard;

extern crate libc;
extern crate nix;

use std::env;
use std::ffi::{CString, OsString};
use std::io::prelude::*;
use std::io;
use std::process;
use std::str::FromStr;

use nix::fcntl::*;
use nix::sys::signal::*;
use nix::sys::stat::*;
use nix::sys::wait::*;
use nix::unistd::*;

mod errors {
    error_chain!{}
}

use errors::*;


quick_main!(|| -> Result<()> { Shell::new().start() });


struct Shell {
    /// Whether the shell should exit its run loop at the next iteration.
    should_exit: bool,
}


#[derive(Debug)]
enum Command {
    /// Execute a command with arguments.
    Exec { prog: CString, argv: Vec<CString> },
    /// Empty command.
    Empty,
    /// Exit.
    Exit,
    /// Change directory.
    Cd(Option<OsString>),
}


/// Ignore SIGTSTP, SIGTTOU, SIGQUIT, SIGTERM.
fn set_signal_handlers() -> Result<()> {
    let sigact = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
    for sig in &[SIGTSTP, SIGTTOU, SIGQUIT, SIGTERM] {
        // This safe because we are not setting a handler function. See
        //   https://github.com/nix-rust/nix/issues/90
        //   http://users.rust-lang.org/t/unix-signals-in-rust/733/3
        unsafe {
            sigaction(*sig, &sigact)
                .chain_err(|| "could not set signal handler")?;
        }
    }
    Ok(())
}


impl Shell {
    /// Create a new `Shell`.
    fn new() -> Shell {
        Shell { should_exit: false }
    }

    /// Start the shell's run loop.
    fn start(&mut self) -> Result<()> {
        // We need to ignore a few signals.
        set_signal_handlers()?;

        // Open the tty device so we can get and set the controlling process
        // group.
        let tty_fd = open("/dev/tty", O_CLOEXEC | O_RDWR, Mode::empty())
            .chain_err(|| "could not open /dev/tty")?;
        defer!(drop(close(tty_fd)));  // drop to ignore result.

        let pid = getpid();
        let original_pgid = getpgrp();
        let original_tpgid = tcgetpgrp(tty_fd)
            .chain_err(|| "could not get terminal's controlling process group")?;

        // Create new process group.
        setpgid(0, 0)
            .chain_err(|| "could not move self to own process group")?;
        defer!(drop(setpgid(0, original_pgid)));  // drop to ignore result.

        // Open the tty device and set the terminal's controlling process group.
        tcsetpgrp(tty_fd, pid)
            .chain_err(|| "could not set terminal's controlling process group")?;
        defer!(drop(tcsetpgrp(tty_fd, original_tpgid)));  // drop to ignore result.

        self.enter_loop()
    }

    fn enter_loop(&mut self) -> Result<()> {
        while !self.should_exit {
            let line = match self.prompt()? {
                Some(l) => l,
                // Exit loop on EOF.
                None => break,
            };
            if let Err(e) = Command::from_str(&line).and_then(|cmd| self.run(cmd)) {
                // Ignore error writing error message.
                let _ = writeln!(io::stderr(), "Error: {}", e);
            }
        }
        // Print `exit` on EOF and exit like bash does :-)
        println!("exit");
        Ok(())
    }

    /// Prompt and return the line entered with newline stripped, or `None` on
    /// EOF.
    fn prompt(&self) -> Result<Option<String>> {
        let mut buf = String::new();
        print!("tiny-shell {}$ ",
               env::current_dir()
                   .chain_err(|| "unable to get current directory")?
                   .display());
        io::stdout()
            .flush()
            .chain_err(|| "unable to write prompt")?;

        let n = io::stdin()
            .read_line(&mut buf)
            .chain_err(|| "unable to read input")?;

        if n == 0 {
            // EOF.
            Ok(None)
        } else {
            // Slightly hackish way to snip the trailing newline on an owned String.
            let len = buf.trim_right().len();
            buf.truncate(len);
            Ok(Some(buf))
        }
    }

    /// Run a command.
    fn run(&mut self, cmd: Command) -> Result<()> {
        match cmd {
            Command::Empty => Ok(()),
            Command::Exit => {
                self.should_exit = true;
                Ok(())
            }
            Command::Cd(dest) => handle_cd(dest),
            Command::Exec { ref prog, ref argv } => handle_exec(prog, argv),
        }
    }
}

fn handle_cd(dest: Option<OsString>) -> Result<()> {
    fn get_home() -> Result<OsString> {
        match env::var_os("HOME") {
            Some(dir) => Ok(dir),
            None => bail!("HOME not set"),
        }
    }

    let dest = dest.unwrap_or(get_home()?);
    env::set_current_dir(dest)
        .chain_err(|| "could not change directory")?;

    Ok(())
}

fn handle_exec(prog: &CString, args: &[CString]) -> Result<()> {
    match fork().chain_err(|| "could not fork")? {
        ForkResult::Parent { child } => {
            waitpid(child, None)
                .chain_err(|| "failed to wait on child")?;
        }
        ForkResult::Child => {
            // TODO: new process group.
            let exe = lookup_exe(prog)?;
            match execve(exe, &args, &[]) {
                // execve does not return successfully!
                Ok(_) => unreachable!(),
                Err(e) => {
                    // Ignore error writing error message.
                    let _ = writeln!(io::stderr(), "execve: {}", e);
                    // Child must exit instead of continuing back to prompt.
                    process::exit(1);
                }
            }
        }
    }
    Ok(())
}

fn lookup_exe(prog: &CString) -> Result<&CString> {
    // TODO: actually lookup prog in PATH.
    Ok(prog)
}

impl std::str::FromStr for Command {
    type Err = Error;

    fn from_str(s: &str) -> Result<Command> {
        let mut parts = s.split_whitespace();
        let cmd = if let Some(c) = parts.next() {
            c
        } else {
            return Ok(Command::Empty);
        };

        match cmd {
            "cd" => Command::parse_cd(parts),
            "exit" => Ok(Command::Exit),
            prog => Command::parse_exec(prog, parts),
        }
    }
}

impl Command {
    /// Parse a `cd` command.
    fn parse_cd<'a, I: Iterator<Item=&'a str>>(mut args: I) -> Result<Command> {
        let dest = args.next().map(|d| OsString::from(d));

        // Check there are no extra args.
        match args.next() {
            Some(_) => bail!("too many args for cd command"),
            None => Ok(Command::Cd(dest)),
        }
    }

    /// Parse an `exec` command.
    fn parse_exec<'a, I: Iterator<Item=&'a str>>(prog: &str, args: I) -> Result<Command> {
        let mut argv = vec![CString::new(prog).chain_err(|| "nul bytes in command")?];

        for a in args {
            argv.push(CString::new(a).chain_err(|| "nul bytes in arg")?);
        }

        Ok(Command::Exec {
            prog: argv[0].clone(),
            argv: argv,
        })
    }
}
