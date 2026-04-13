use crate::terminal::engine::{PtyReadResult, Terminal};
use nix::libc;
use nix::unistd::Pid;
use std::os::fd::RawFd;
use std::path::PathBuf;

pub struct Pty {
    master_fd: RawFd,
    child_pid: Pid,
}

pub fn pty_write_raw(fd: i32, data: &[u8]) {
    let mut buf = data;
    while !buf.is_empty() {
        let n = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if n > 0 {
            buf = &buf[n as usize..];
        } else if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
    }
}

impl Pty {
    pub fn spawn(cols: u16, rows: u16, cw: i32, ch: i32) -> Result<Self, String> {
        unsafe {
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: (cols as u16).wrapping_mul(cw as u16),
                ws_ypixel: (rows as u16).wrapping_mul(ch as u16),
            };

            let mut master_fd: i32 = -1;
            let pid = libc::forkpty(
                &mut master_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &ws as *const _ as *mut _,
            );

            if pid < 0 {
                return Err("forkpty failed".to_string());
            }

            if pid == 0 {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| {
                    let pw = libc::getpwuid(libc::getuid());
                    if !pw.is_null() && !(*pw).pw_shell.is_null() {
                        let c_str = std::ffi::CStr::from_ptr((*pw).pw_shell);
                        c_str.to_string_lossy().to_string()
                    } else {
                        "/bin/sh".to_string()
                    }
                });

                let shell_name = shell.rsplit('/').next().unwrap_or(&shell);

                for var in &["NO_COLOR", "FORCE_COLOR", "CI"] {
                    let key = std::ffi::CString::new(*var).unwrap();
                    libc::unsetenv(key.as_ptr());
                }

                let env_vars: &[(&str, &str)] = &[
                    ("TERM", "xterm-256color"),
                    ("COLORTERM", "truecolor"),
                    ("TERM_PROGRAM", "tai"),
                    ("HISTCONTROL", "ignorespace"),
                ];
                for (k, v) in env_vars {
                    let key = std::ffi::CString::new(*k).unwrap();
                    let val = std::ffi::CString::new(*v).unwrap();
                    libc::setenv(key.as_ptr(), val.as_ptr(), 1);
                }

                let shell_c = std::ffi::CString::new(shell.as_str()).unwrap();
                let name_c = std::ffi::CString::new(shell_name).unwrap();

                if shell_name == "zsh" {
                    let opt_o1 = std::ffi::CString::new("-o").unwrap();
                    let val1 = std::ffi::CString::new("HIST_IGNORE_SPACE").unwrap();
                    let opt_o2 = std::ffi::CString::new("-o").unwrap();
                    let val2 = std::ffi::CString::new("NO_BANG_HIST").unwrap();
                    let args: Vec<*const libc::c_char> = vec![
                        name_c.as_ptr(),
                        opt_o1.as_ptr(),
                        val1.as_ptr(),
                        opt_o2.as_ptr(),
                        val2.as_ptr(),
                        std::ptr::null(),
                    ];
                    libc::execv(shell_c.as_ptr(), args.as_ptr());
                } else {
                    libc::execl(
                        shell_c.as_ptr(),
                        name_c.as_ptr(),
                        std::ptr::null::<libc::c_char>(),
                    );
                }
                libc::_exit(127);
            }

            let flags = libc::fcntl(master_fd, libc::F_GETFL);
            if flags < 0 || libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                libc::close(master_fd);
                return Err("Failed to set O_NONBLOCK".to_string());
            }

            Ok(Pty {
                master_fd,
                child_pid: Pid::from_raw(pid),
            })
        }
    }

    pub fn read_nonblocking(
        &self,
        terminal: &mut Terminal,
        mut capture: Option<&mut Vec<u8>>,
        mut mirror: Option<&mut Vec<u8>>,
    ) -> PtyReadResult {
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe {
                libc::read(
                    self.master_fd,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n > 0 {
                let bytes = &buf[..n as usize];
                terminal.vt_write(bytes);
                if let Some(ref mut cap) = capture {
                    cap.extend_from_slice(bytes);
                }
                if let Some(ref mut m) = mirror {
                    m.extend_from_slice(bytes);
                }
            } else if n == 0 {
                return PtyReadResult::Eof;
            } else {
                let err = std::io::Error::last_os_error();
                match err.raw_os_error().unwrap_or(0) {
                    libc::EAGAIN => return PtyReadResult::Ok,
                    libc::EINTR => continue,
                    libc::EIO => return PtyReadResult::Eof,
                    _ => return PtyReadResult::Error,
                }
            }
        }
    }

    pub fn write(&self, data: &[u8]) {
        pty_write_raw(self.master_fd, data);
    }

    pub fn resize(&self, cols: u16, rows: u16, cw: i32, ch: i32) {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: (cols as u16).wrapping_mul(cw as u16),
            ws_ypixel: (rows as u16).wrapping_mul(ch as u16),
        };
        unsafe {
            libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &ws);
        }
    }

    pub fn get_cwd(&self) -> Option<PathBuf> {
        let pid = self.get_foreground_pid().unwrap_or(self.child_pid);
        libproc::proc_pid::pidcwd(pid.as_raw()).ok()
            .or_else(|| std::env::current_dir().ok())
    }

    pub fn get_foreground_process_name(&self) -> Option<String> {
        let fg_pid = self.get_foreground_pid()?;
        if fg_pid == self.child_pid {
            return None; // Shell itself, no special process
        }
        libproc::proc_pid::name(fg_pid.as_raw()).ok()
    }

    pub fn get_foreground_pid(&self) -> Option<Pid> {
        unsafe {
            let mut pgrp: libc::pid_t = 0;
            let ret = libc::ioctl(self.master_fd, libc::TIOCGPGRP, &mut pgrp);
            if ret == 0 && pgrp > 0 {
                Some(Pid::from_raw(pgrp))
            } else {
                None
            }
        }
    }

    pub fn set_echo(&self, enable: bool) {
        unsafe {
            let mut attrs: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(self.master_fd, &mut attrs) == 0 {
                if enable {
                    attrs.c_lflag |= libc::ECHO;
                } else {
                    attrs.c_lflag &= !libc::ECHO;
                }
                libc::tcsetattr(self.master_fd, libc::TCSANOW, &attrs);
            }
        }
    }

    pub fn master_fd(&self) -> RawFd {
        self.master_fd
    }

    pub fn child_pid(&self) -> Pid {
        self.child_pid
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.master_fd);
            libc::kill(self.child_pid.as_raw(), libc::SIGHUP);
            libc::waitpid(self.child_pid.as_raw(), std::ptr::null_mut(), libc::WNOHANG);
        }
    }
}
