use crate::terminal::engine::{PtyReadResult, Terminal};
use crate::terminal::pty::{Pty, pty_write_raw};
use crate::terminal::ssh::SshBackend;
use nix::unistd::Pid;
use std::os::unix::io::RawFd;
use std::path::PathBuf;

pub enum Backend {
    Local(Pty),
    Ssh(SshBackend),
}

impl Backend {
    pub fn get_cwd(&self) -> Option<PathBuf> {
        match self {
            Backend::Local(pty) => pty.get_cwd(),
            Backend::Ssh(_) => None,
        }
    }

    pub fn get_foreground_process_name(&self) -> Option<String> {
        match self {
            Backend::Local(pty) => pty.get_foreground_process_name(),
            Backend::Ssh(_) => None,
        }
    }

    pub fn master_fd(&self) -> RawFd {
        match self {
            Backend::Local(pty) => pty.master_fd(),
            Backend::Ssh(ssh) => ssh.proxy_fd(),
        }
    }

    pub fn child_pid(&self) -> Pid {
        match self {
            Backend::Local(pty) => pty.child_pid(),
            Backend::Ssh(_) => Pid::from_raw(0),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Backend::Local(_))
    }

    pub fn read_nonblocking(
        &mut self,
        terminal: &mut Terminal,
        capture: Option<&mut Vec<u8>>,
        mirror: Option<&mut Vec<u8>>,
    ) -> PtyReadResult {
        match self {
            Backend::Local(pty) => pty.read_nonblocking(terminal, capture, mirror),
            Backend::Ssh(ssh) => ssh.read_nonblocking(terminal, capture, mirror),
        }
    }

    pub fn write(&mut self, data: &[u8]) {
        match self {
            Backend::Local(pty) => pty_write_raw(pty.master_fd(), data),
            Backend::Ssh(ssh) => ssh.write(data),
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cw: i32, ch: i32) {
        match self {
            Backend::Local(pty) => pty.resize(cols, rows, cw, ch),
            Backend::Ssh(ssh) => ssh.resize(cols, rows),
        }
    }

    pub fn set_echo(&mut self, enable: bool) {
        match self {
            Backend::Local(pty) => pty.set_echo(enable),
            Backend::Ssh(_) => {}
        }
    }
}
