use nix::sys::ptrace;
use nix::sys::ptrace::{cont, getregs};
use nix::sys::signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::mem::size_of;
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::Command;

use crate::dwarf_data::DwarfData;

fn align_addr_to_word(addr: usize) -> usize {
    addr & (-(size_of::<usize>() as isize) as usize)
}

#[derive(Debug)]
pub enum Status {
    /// Indicates inferior stopped. Contains the signal that stopped the process, as well as the
    /// current instruction pointer that it is stopped at.
    Stopped(signal::Signal, usize),

    /// Indicates inferior exited normally. Contains the exit status code.
    Exited(i32),

    /// Indicates the inferior exited due to a signal. Contains the signal that killed the
    /// process.
    Signaled(signal::Signal),
}

/// This function calls ptrace with PTRACE_TRACEME to enable debugging on a process. You should use
/// pre_exec with Command to call this in the child process.
fn child_traceme() -> Result<(), std::io::Error> {
    ptrace::traceme().or(Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "ptrace TRACEME failed",
    )))
}

pub struct Inferior {
    child: Child,
}

impl Inferior {
    /// Attempts to start a new inferior process. Returns Some(Inferior) if successful, or None if
    /// an error is encountered.
    pub fn new(target: &str, args: &Vec<String>) -> Option<Inferior> {
        let mut command = Command::new(target);
        command.args(args);
        unsafe {
            command.pre_exec(child_traceme); // fn child_traceme will run before exce
        }
        let child = command.spawn().ok()?; // child/inferior will *pause* because PTRACE_TRACEME
        let inferior = Inferior { child };

        Some(inferior)
    }

    /// Returns the pid of this inferior.
    pub fn pid(&self) -> Pid {
        nix::unistd::Pid::from_raw(self.child.id() as i32)
    }

    /// Calls waitpid on this inferior and returns a Status to indicate the state of the process
    /// after the waitpid call.
    pub fn wait(&self, options: Option<WaitPidFlag>) -> Result<Status, nix::Error> {
        Ok(match waitpid(self.pid(), options)? {
            WaitStatus::Exited(_pid, exit_code) => Status::Exited(exit_code),
            WaitStatus::Signaled(_pid, signal, _core_dumped) => Status::Signaled(signal),
            WaitStatus::Stopped(_pid, signal) => {
                let regs = ptrace::getregs(self.pid())?;
                Status::Stopped(signal, regs.rip as usize)
            }
            other => panic!("waitpid returned unexpected status: {:?}", other),
        })
    }

    pub fn continue_inferior(&self) -> Result<Status, nix::Error> {
        cont(self.pid(), None)?; // wake inferior
        self.wait(None) // wait inferior
    }

    pub fn step(&self) -> Result<Status, nix::Error> {
        ptrace::step(self.pid(), None)?;
        self.wait(None) // wait inferior
    }

    pub fn back_rip(&mut self) -> Result<(), nix::Error> {
        let mut regs = getregs(self.pid()).unwrap();
        regs.rip -= 1;
        ptrace::setregs(self.pid(), regs)
    }

    pub fn rip(&self) -> usize {
        getregs(self.pid()).expect("get rip error").rip as usize
    }

    pub fn write_byte(&mut self, addr: usize, val: u8) -> Result<u8, nix::Error> {
        let aligned_addr = align_addr_to_word(addr);
        let byte_offset = addr - aligned_addr;
        let word = ptrace::read(self.pid(), aligned_addr as ptrace::AddressType)? as u64;
        let orig_byte = (word >> 8 * byte_offset) & 0xff;
        let masked_word = word & !(0xff << 8 * byte_offset);
        let updated_word = masked_word | ((val as u64) << 8 * byte_offset);
        ptrace::write(
            self.pid(),
            aligned_addr as ptrace::AddressType,
            updated_word as *mut std::ffi::c_void,
        )?;
        Ok(orig_byte as u8)
    }

    pub fn print(&self, status: &Status, debug_data: &DwarfData) {
        match status {
            Status::Exited(exit_code) => println!("Child exit (status {}) ", exit_code),
            Status::Stopped(signal, line) => {
                println!("Child stop (signal {})", signal);
                match debug_data.get_line_from_addr(*line) {
                    Some(location) => println!("Stopped at {}", location),
                    None => (),
                }
            }
            Status::Signaled(signal) => println!("signal: {}", signal),
        }
    }

    pub fn kill(&mut self) {
        self.child.kill().expect("kill process failed");
        println!("kill running inferior (pid {})", self.pid());
    }

    pub fn print_backtrace(&self, debug_data: &DwarfData) -> Result<(), nix::Error> {
        let pid = self.pid();
        let rip = getregs(pid)?.rip;
        let mut rbp = getregs(pid)?.rbp;

        let mut instruction_ptr = rip as usize;
        let mut base_ptr = rbp as usize;

        let mut line;
        let mut fn_name;

        loop {
            line = debug_data.get_line_from_addr(instruction_ptr).unwrap();
            fn_name = debug_data.get_function_from_addr(instruction_ptr).unwrap();
            println!("{} {}", fn_name, line);

            if fn_name == "main".to_string() {
                break;
            }

            rbp += 8;
            instruction_ptr = ptrace::read(pid, (base_ptr + 8) as ptrace::AddressType)? as usize;
            base_ptr = ptrace::read(pid, base_ptr as ptrace::AddressType)? as usize;
        }

        Ok(())
    }
}
