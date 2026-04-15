use crate::terminal::engine::{PtyReadResult, Terminal};
use nix::libc;
use serde::{Deserialize, Serialize};
use ssh2::KeyboardInteractivePrompt;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::io::RawFd;
use std::time::Duration;

fn ssh_log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/tai-ssh-debug.log")
    {
        use std::io::Write as _;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let _ = writeln!(f, "[{:.3}] {}", now.as_secs_f64(), msg);
    }
}

pub struct SshBackend {
    channel: ssh2::Channel,
    proxy_fd: RawFd,
    bridge_fd: RawFd,
    pub info: SshTabInfo,
}

impl SshBackend {
    pub fn new(channel: ssh2::Channel, proxy_fd: RawFd, bridge_fd: RawFd, info: SshTabInfo) -> Self {
        SshBackend { channel, proxy_fd, bridge_fd, info }
    }

    pub fn proxy_fd(&self) -> RawFd {
        self.proxy_fd
    }

    pub fn read_nonblocking(
        &mut self,
        terminal: &mut Terminal,
        mut capture: Option<&mut Vec<u8>>,
        mut mirror: Option<&mut Vec<u8>>,
    ) -> PtyReadResult {
        let mut bridge_buf = [0u8; 4096];
        unsafe {
            loop {
                let n = libc::read(self.bridge_fd, bridge_buf.as_mut_ptr().cast(), bridge_buf.len());
                if n > 0 {
                    ssh_log(&format!("bridge->channel: {} bytes", n));
                    let _ = self.channel.write_all(&bridge_buf[..n as usize]);
                } else {
                    break;
                }
            }
        }

        let mut buf = [0u8; 4096];
        loop {
            match self.channel.read(&mut buf) {
                Ok(0) => {
                    ssh_log("channel read: EOF (0 bytes)");
                    return PtyReadResult::Eof;
                }
                Ok(n) => {
                    let bytes = &buf[..n];
                    ssh_log(&format!("channel->vt: {} bytes", n));
                    terminal.vt_write(bytes);
                    if let Some(ref mut cap) = capture {
                        cap.extend_from_slice(bytes);
                    }
                    if let Some(ref mut m) = mirror {
                        m.extend_from_slice(bytes);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return PtyReadResult::Ok,
                Err(e) => {
                    ssh_log(&format!("channel read error: {e}"));
                    if self.channel.eof() {
                        ssh_log("channel is EOF after error");
                        return PtyReadResult::Eof;
                    }
                    return PtyReadResult::Error;
                }
            }
        }
    }

    pub fn write(&mut self, data: &[u8]) {
        let _ = self.channel.write_all(data);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.channel.request_pty_size(cols as u32, rows as u32, None, None);
    }
}

impl Drop for SshBackend {
    fn drop(&mut self) {
        ssh_log(&format!("dropping SshBackend for {}@{}:{}", self.info.user, self.info.host, self.info.port));
        let _ = self.channel.close();
        unsafe {
            libc::close(self.proxy_fd);
            libc::close(self.bridge_fd);
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SshTabInfo {
    pub host: String,
    pub port: u16,
    pub user: String,
}

struct PasswordPrompt<'a>(&'a str);

impl KeyboardInteractivePrompt for PasswordPrompt<'_> {
    fn prompt<'a>(
        &mut self,
        _username: &str,
        _instructions: &str,
        prompts: &[ssh2::Prompt<'a>],
    ) -> Vec<String> {
        prompts.iter().map(|_| self.0.to_string()).collect()
    }
}

pub struct SshConnectionManager {
    connections: HashMap<String, ssh2::Session>,
}

impl SshConnectionManager {
    pub fn new() -> Self {
        SshConnectionManager { connections: HashMap::new() }
    }

    pub fn get_or_connect(
        &mut self,
        host: &str,
        port: u16,
        user: &str,
        password: &str,
    ) -> Result<ssh2::Session, String> {
        let key = format!("{}@{}:{}", user, host, port);
        if let Some(sess) = self.connections.get(&key) {
            ssh_log(&format!("reusing existing session for {key}"));
            return Ok(sess.clone());
        }
        ssh_log(&format!("opening new connection to {key}"));

        use std::net::ToSocketAddrs;
        let addr = (host, port)
            .to_socket_addrs()
            .map_err(|e| format!("DNS resolution failed: {e}"))?
            .next()
            .ok_or_else(|| format!("No addresses found for {host}"))?;

        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .map_err(|e| format!("Connection failed: {e}"))?;
        let mut sess = ssh2::Session::new().map_err(|e| e.to_string())?;
        sess.set_tcp_stream(tcp);
        sess.handshake().map_err(|e| format!("SSH handshake failed: {e}"))?;

        if sess.userauth_agent(user).is_err() {
            let mut prompter = PasswordPrompt(password);
            if sess.userauth_keyboard_interactive(user, &mut prompter).is_err() {
                sess.userauth_password(user, password)
                    .map_err(|e| format!("Authentication failed: {e}"))?;
            }
        }

        ssh_log(&format!("authenticated successfully as {user}@{host}:{port}"));
        self.connections.insert(key, sess.clone());
        Ok(sess)
    }

    pub fn remove(&mut self, host: &str, port: u16, user: &str) {
        let key = format!("{}@{}:{}", user, host, port);
        self.connections.remove(&key);
    }

    pub fn clear(&mut self) {
        self.connections.clear();
    }

    pub fn open_channel(
        &self,
        session: &ssh2::Session,
        cols: u16,
        rows: u16,
        info: SshTabInfo,
    ) -> Result<SshBackend, String> {
        ssh_log(&format!("open_channel: {}x{} for {}@{}:{}", cols, rows, info.user, info.host, info.port));
        session.set_blocking(true);
        let channel_result = (|| -> Result<ssh2::Channel, String> {
            let mut ch = session.channel_session().map_err(|e| e.to_string())?;
            ch.request_pty("xterm-256color", None, Some((cols as u32, rows as u32, 0, 0)))
                .map_err(|e| e.to_string())?;
            ch.shell().map_err(|e| e.to_string())?;
            Ok(ch)
        })();
        session.set_blocking(false);
        let channel = channel_result?;

        let mut fds = [0i32; 2];
        if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) } != 0 {
            return Err("socketpair failed".into());
        }
        unsafe {
            libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK);
        }

        Ok(SshBackend::new(channel, fds[0], fds[1], info))
    }
}
