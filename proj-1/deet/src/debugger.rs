use crate::debugger_command::DebuggerCommand;
use crate::dwarf_data::{DwarfData, Error as DwarfError};
use crate::inferior::Inferior;
use rustyline::error::ReadlineError;
use rustyline::Editor;

pub struct Debugger {
    target: String,
    history_path: String,
    readline: Editor<()>,
    inferior: Option<Inferior>,
    debug_data: DwarfData,
    breakpoints: Vec<(usize, u8)>,
}

impl Debugger {
    /// Initializes the debugger.
    pub fn new(target: &str) -> Debugger {
        let debug_data = match DwarfData::from_file(target) {
            Ok(val) => val,
            Err(DwarfError::ErrorOpeningFile) => {
                println!("Could not open file {}", target);
                std::process::exit(1);
            }
            Err(DwarfError::DwarfFormatError(err)) => {
                println!("Could not debugging symbols from {}: {:?}", target, err);
                std::process::exit(1);
            }
        };

        let history_path = format!("{}/.deet_history", std::env::var("HOME").unwrap());
        let mut readline = Editor::<()>::new();
        // Attempt to load history from ~/.deet_history if it exists
        let _ = readline.load_history(&history_path);

        let breakpoints = Vec::new();

        debug_data.print();

        Debugger {
            target: target.to_string(),
            history_path,
            readline,
            inferior: None,
            debug_data,
            breakpoints,
        }
    }

    pub fn run(&mut self) {
        loop {
            match self.get_next_command() {
                DebuggerCommand::Run(args) => {
                    if self.inferior.is_some() {
                        self.inferior.as_mut().unwrap().kill();
                        self.inferior = None;
                    }

                    if let Some(inferior) = Inferior::new(&self.target, &args) {
                        // Create the inferior
                        self.inferior = Some(inferior);

                        self.update_breakpoint();

                        let status = self.inferior.as_mut().unwrap().continue_inferior().unwrap();
                        self.inferior
                            .as_mut()
                            .unwrap()
                            .print(&status, &self.debug_data);
                    } else {
                        println!("Error starting subprocess");
                    }
                }
                DebuggerCommand::Quit => {
                    if self.inferior.is_some() {
                        self.inferior.as_mut().unwrap().kill();
                    }
                    return;
                }
                DebuggerCommand::Cont => match self.inferior.as_mut() {
                    None => {
                        println!("Error: can't use cont when no process running!");
                    }
                    Some(inferior) => {
                        let rip = inferior.rip();
                        match self
                            .breakpoints
                            .iter()
                            .find(|(addr, _val)| rip - 1 == *addr)
                        {
                            Some((addr, val)) => {
                                inferior.write_byte(*addr, *val).expect("0xcc -> val error");
                                inferior.back_rip().unwrap();
                                inferior.step().unwrap();
                                inferior.write_byte(*addr, 0xcc).expect("val -> 0xcc error");
                            }
                            _ => (),
                        }

                        let status = inferior.continue_inferior().unwrap();
                        inferior.print(&status, &self.debug_data);

                        match status {
                            crate::inferior::Status::Exited(_) => {
                                self.inferior = None;
                            }
                            _ => (),
                        }
                    }
                },
                DebuggerCommand::Back => {
                    self.inferior
                        .as_mut()
                        .unwrap()
                        .print_backtrace(&self.debug_data)
                        .unwrap();
                }
                DebuggerCommand::Break(address) => {
                    let addr = parse_address(&address, &self.debug_data).unwrap();

                    if self.inferior.is_some() {
                        let ori_ins = self
                            .inferior
                            .as_mut()
                            .unwrap()
                            .write_byte(addr, 0xcc)
                            .expect("invalid address");

                        self.breakpoints.push((addr, ori_ins));
                    } else {
                        self.breakpoints.push((addr, 0));
                    }

                    println!(
                        "Set breakpoint {} at {:#x}",
                        self.breakpoints.len() - 1,
                        addr
                    );
                }
            }
        }
    }

    /// This function prompts the user to enter a command, and continues re-prompting until the user
    /// enters a valid command. It uses DebuggerCommand::from_tokens to do the command parsing.
    ///
    /// You don't need to read, understand, or modify this function.
    fn get_next_command(&mut self) -> DebuggerCommand {
        loop {
            // Print prompt and get next line of user input
            match self.readline.readline("(deet) ") {
                Err(ReadlineError::Interrupted) => {
                    // User pressed ctrl+c. We're going to ignore it
                    println!("Type \"quit\" to exit");
                }
                Err(ReadlineError::Eof) => {
                    // User pressed ctrl+d, which is the equivalent of "quit" for our purposes
                    return DebuggerCommand::Quit;
                }
                Err(err) => {
                    panic!("Unexpected I/O error: {:?}", err);
                }
                Ok(line) => {
                    if line.trim().len() == 0 {
                        continue;
                    }
                    self.readline.add_history_entry(line.as_str());
                    if let Err(err) = self.readline.save_history(&self.history_path) {
                        println!(
                            "Warning: failed to save history file at {}: {}",
                            self.history_path, err
                        );
                    }
                    let tokens: Vec<&str> = line.split_whitespace().collect();
                    if let Some(cmd) = DebuggerCommand::from_tokens(&tokens) {
                        return cmd;
                    } else {
                        println!("Unrecognized command.");
                    }
                }
            }
        }
    }

    fn update_breakpoint(&mut self) {
        let mut new_breaks = Vec::new();
        if !self.breakpoints.is_empty() {
            for (addr, _) in self.breakpoints.clone() {
                let ori_ins = self
                    .inferior
                    .as_mut()
                    .unwrap()
                    .write_byte(addr, 0xcc)
                    .expect("invalid address");
                new_breaks.push((addr, ori_ins));
            }

            self.breakpoints = new_breaks;
        }
    }
}

fn parse_address(addr: &str, dwarfdata: &DwarfData) -> Option<usize> {
    match addr.parse::<usize>() {
        Ok(line_number) => return dwarfdata.get_addr_for_line(None, line_number),
        _ => (),
    }

    if !addr.starts_with("*") {
        return dwarfdata.get_addr_for_function(None, addr);
    }

    let addr = &addr[1..];
    let addr_without_0x = if addr.to_lowercase().starts_with("0x") {
        &addr[2..]
    } else {
        &addr
    };

    usize::from_str_radix(addr_without_0x, 16).ok()
}
