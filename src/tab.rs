use crate::ai::bridge::AiBridge;
use crate::config::TaiConfig;
use crate::minimap::Minimap;
use crate::overlay::CommandOverlay;
use crate::router::InputRouter;
use crate::selection::TextSelection;
use crate::session::{SessionRouter, session_messages_to_api};
use crate::terminal::backend::Backend;
use crate::terminal::engine::{PtyReadResult, Terminal};
use crate::terminal::pty::Pty;
use crate::terminal::ssh::{SshBackend, SshTabInfo};
use std::path::PathBuf;

pub struct TabSession {
    pub term: Terminal,
    pub backend: Backend,
    pub minimap: Minimap,
    pub router: InputRouter,
    pub selection: TextSelection,
    pub overlay: CommandOverlay,
    pub pty_mirror: Vec<u8>,
    pub child_exited: bool,
    pub ssh_info: Option<SshTabInfo>,
    pending_scrollback: Option<Vec<String>>,
    prompt_ready_time: Option<std::time::Instant>,
    /// Raw bytes of the last complete prompt render (PROMPT_SP + prompt),
    /// captured so we can replay a single clean prompt after injection.
    last_prompt_bytes: Vec<u8>,
    /// Original scrollback text from session load, used to prevent
    /// auto-save from accumulating injected content.
    pub original_scrollback: Option<String>,
}

impl TabSession {
    pub fn new(config: &TaiConfig, cols: u16, rows: u16, cw: i32, ch: i32) -> Result<Self, String> {
        let mut term = Terminal::new(cols, rows, config.terminal.scrollback)?;
        let pty = Pty::spawn(cols, rows, cw, ch)?;
        term.setup_effects(pty.master_fd(), cw, ch);
        term.resize(cols, rows, cw as u32, ch as u32);

        let ai_bridge = config.api_key().map(|key| AiBridge::new(&config.ai, &key));
        let router = InputRouter::new(config, ai_bridge);

        Ok(TabSession {
            term,
            backend: Backend::Local(pty),
            minimap: Minimap::new(cols),
            router,
            selection: TextSelection::new(),
            overlay: CommandOverlay::new(),
            pty_mirror: Vec::with_capacity(8192),
            child_exited: false,
            ssh_info: None,
            pending_scrollback: None,
            prompt_ready_time: None,
            last_prompt_bytes: Vec::new(),
            original_scrollback: None,
        })
    }

    pub fn new_in_dir(
        config: &TaiConfig,
        cwd: PathBuf,
        cols: u16,
        rows: u16,
        cw: i32,
        ch: i32,
        scrollback: &str,
        _scroll_offset: u64,
        router_state: &SessionRouter,
    ) -> Result<Self, String> {
        let mut term = Terminal::new(cols, rows, config.terminal.scrollback)?;
        term.resize(cols, rows, cw as u32, ch as u32);

        let scrollback_lines: Vec<&str> = scrollback.lines().collect();

        let pty = Pty::spawn_in_dir(cwd.clone(), cols, rows, cw, ch)?;
        term.setup_effects(pty.master_fd(), cw, ch);

        let (pending, orig) = if !scrollback.is_empty() {
            (
                Some(scrollback_lines.iter().map(|s| s.to_string()).collect()),
                Some(scrollback.to_string()),
            )
        } else {
            (None, None)
        };

        let ai_bridge = config.api_key().map(|key| AiBridge::new(&config.ai, &key));
        let mut router = InputRouter::new(config, ai_bridge);
        let messages = session_messages_to_api(&router_state.conversation);
        router.restore_history(
            router_state.unified_history.clone(),
            router_state.command_history.clone(),
            router_state.auto_execute,
        );
        router.restore_conversation(messages);

        Ok(TabSession {
            term,
            backend: Backend::Local(pty),
            minimap: Minimap::new(cols),
            router,
            selection: TextSelection::new(),
            overlay: CommandOverlay::new(),
            pty_mirror: Vec::with_capacity(8192),
            child_exited: false,
            ssh_info: None,
            pending_scrollback: pending,
            prompt_ready_time: None,
            last_prompt_bytes: Vec::new(),
            original_scrollback: orig,
        })
    }

    pub fn new_dead(
        config: &TaiConfig,
        title: &str,
        scrollback: &str,
        scroll_offset: u64,
        cols: u16,
        rows: u16,
        cw: i32,
        ch: i32,
        router_state: &SessionRouter,
    ) -> Result<Self, String> {
        let mut term = Terminal::new(cols, rows, config.terminal.scrollback)?;
        let pty = Pty::null();
        term.resize(cols, rows, cw as u32, ch as u32);

        if !scrollback.is_empty() {
            for line in scrollback.lines() {
                let mut buf = line.as_bytes().to_vec();
                buf.extend_from_slice(b"\r\n");
                term.vt_write(&buf);
            }
            term.drain_vt_mirror();
        }

        term.setup_effects(pty.master_fd(), cw, ch);

        if let Some((total, _offset, len)) = term.get_scrollbar() {
            let current_bottom = total.saturating_sub(len) as i64;
            let delta = scroll_offset as i64 - current_bottom;
            if delta != 0 {
                term.scroll_viewport(delta as i32);
            }
        }

        *term.last_osc_title = title.to_string();

        let ai_bridge = config.api_key().map(|key| AiBridge::new(&config.ai, &key));
        let mut router = InputRouter::new(config, ai_bridge);
        let messages = session_messages_to_api(&router_state.conversation);
        router.restore_history(
            router_state.unified_history.clone(),
            router_state.command_history.clone(),
            router_state.auto_execute,
        );
        router.restore_conversation(messages);

        let mut minimap = Minimap::new(cols);
        let buffer_text = term.get_buffer_text(0);
        minimap.rebuild_from_text(&buffer_text);

        Ok(TabSession {
            term,
            backend: Backend::Local(pty),
            minimap,
            router,
            selection: TextSelection::new(),
            overlay: CommandOverlay::new(),
            pty_mirror: Vec::with_capacity(8192),
            child_exited: true,
            ssh_info: None,
            pending_scrollback: None,
            prompt_ready_time: None,
            last_prompt_bytes: Vec::new(),
            original_scrollback: None,
        })
    }

    pub fn new_ssh(
        config: &TaiConfig,
        ssh_backend: SshBackend,
        cols: u16,
        rows: u16,
        cw: i32,
        ch: i32,
    ) -> Result<Self, String> {
        let mut term = Terminal::new(cols, rows, config.terminal.scrollback)?;
        term.resize(cols, rows, cw as u32, ch as u32);

        let ssh_info = Some(ssh_backend.info.clone());
        term.setup_effects(ssh_backend.proxy_fd(), cw, ch);

        let ai_bridge = config.api_key().map(|key| AiBridge::new(&config.ai, &key));
        let router = InputRouter::new(config, ai_bridge);

        Ok(TabSession {
            term,
            backend: Backend::Ssh(ssh_backend),
            minimap: Minimap::new(cols),
            router,
            selection: TextSelection::new(),
            overlay: CommandOverlay::new(),
            pty_mirror: Vec::with_capacity(8192),
            child_exited: false,
            ssh_info,
            pending_scrollback: None,
            prompt_ready_time: None,
            last_prompt_bytes: Vec::new(),
            original_scrollback: None,
        })
    }

    pub fn read_pty(&mut self) {
        if self.child_exited {
            return;
        }
        let before = self.pty_mirror.len();
        let capture = self.router.capture_buffer();
        match self.backend.read_nonblocking(&mut self.term, capture, Some(&mut self.pty_mirror)) {
            PtyReadResult::Ok => {}
            PtyReadResult::Eof | PtyReadResult::Error => {
                self.child_exited = true;
            }
        }

        if self.pending_scrollback.is_some() {
            let new_data = &self.pty_mirror[before..];
            if !new_data.is_empty() {
                if new_data.windows(8).any(|w| w == b"\x1b[?2004h") {
                    self.last_prompt_bytes = self.pty_mirror[before..].to_vec();
                    self.prompt_ready_time = Some(std::time::Instant::now());
                } else {
                    self.prompt_ready_time = None;
                }
            }
        }

        if !self.pty_mirror.is_empty() {
            self.minimap.feed(&self.pty_mirror);
            self.pty_mirror.clear();
        }

        if self.pending_scrollback.is_some() {
            if let Some(ready) = self.prompt_ready_time {
                if ready.elapsed() >= std::time::Duration::from_millis(200) {
                    self.inject_pending_scrollback();
                }
            }
        }
    }

    fn inject_pending_scrollback(&mut self) {
        let lines = match self.pending_scrollback.take() {
            Some(l) if !l.is_empty() => l,
            _ => return,
        };

        self.term.vt_write(b"\x1b[H\x1b[J");

        for line in &lines {
            let mut buf = line.as_bytes().to_vec();
            buf.extend_from_slice(b"\r\n");
            self.term.vt_write(&buf);
        }
        self.term.vt_write(b"\x1b[0m");

        if !self.last_prompt_bytes.is_empty() {
            let prompt = self.last_prompt_bytes.clone();
            self.term.vt_write(&prompt);
        }

        self.term.drain_vt_mirror();

        self.original_scrollback = None;
    }

    pub fn poll_ai(&mut self) {
        self.router.poll_ai_responses(&mut self.term, &mut self.backend, &mut self.overlay);
        if let Some(vt_data) = self.term.drain_vt_mirror() {
            self.minimap.feed(&vt_data);
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cw: u32, ch: u32) {
        self.term.resize(cols, rows, cw, ch);
        self.backend.resize(cols, rows, cw as i32, ch as i32);
        self.minimap.set_cols(cols);
        let buffer_text = self.term.get_buffer_text(0);
        self.minimap.rebuild_from_text(&buffer_text);
    }

    pub fn revive_ssh(&mut self, ssh_backend: SshBackend, cw: i32, ch: i32) {
        let info = ssh_backend.info.clone();
        self.term.setup_effects(ssh_backend.proxy_fd(), cw, ch);
        self.backend = Backend::Ssh(ssh_backend);
        self.ssh_info = Some(info);
        self.child_exited = false;
        let title = &*self.term.last_osc_title;
        if title.ends_with("(disconnected)") {
            *self.term.last_osc_title = String::new();
        }
    }

    pub fn title(&self) -> String {
        if !self.term.last_osc_title.is_empty() {
            return (*self.term.last_osc_title).clone();
        }
        if let Some(ref info) = self.ssh_info {
            return format!("{}@{}", info.user, info.host);
        }
        match self.backend.get_foreground_process_name() {
            Some(name) => name,
            None => {
                self.backend.get_cwd()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "shell".to_string())
            }
        }
    }
}
