use crate::ai::bridge::AiBridge;
use crate::config::TaiConfig;
use crate::minimap::Minimap;
use crate::overlay::CommandOverlay;
use crate::router::InputRouter;
use crate::selection::TextSelection;
use crate::session::{SessionRouter, session_messages_to_api};
use crate::terminal::engine::{PtyReadResult, Terminal};
use crate::terminal::pty::Pty;
use std::path::PathBuf;

pub struct TabSession {
    pub term: Terminal,
    pub pty: Pty,
    pub minimap: Minimap,
    pub router: InputRouter,
    pub selection: TextSelection,
    pub overlay: CommandOverlay,
    pub pty_mirror: Vec<u8>,
    pub child_exited: bool,
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
            pty,
            minimap: Minimap::new(cols),
            router,
            selection: TextSelection::new(),
            overlay: CommandOverlay::new(),
            pty_mirror: Vec::with_capacity(8192),
            child_exited: false,
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
        scroll_offset: u64,
        router_state: &SessionRouter,
    ) -> Result<Self, String> {
        let mut term = Terminal::new(cols, rows, config.terminal.scrollback)?;
        let pty = Pty::spawn_in_dir(cwd, cols, rows, cw, ch)?;
        term.resize(cols, rows, cw as u32, ch as u32);

        // Feed scrollback BEFORE setup_effects so pty_fd is not yet wired
        if !scrollback.is_empty() {
            for line in scrollback.lines() {
                let mut buf = line.as_bytes().to_vec();
                buf.extend_from_slice(b"\r\n");
                term.vt_write(&buf);
            }
            term.drain_vt_mirror();
        }

        term.setup_effects(pty.master_fd(), cw, ch);

        // Restore scroll position
        if let Some((total, _offset, len)) = term.get_scrollbar() {
            let current_bottom = total.saturating_sub(len) as i64;
            let delta = scroll_offset as i64 - current_bottom;
            if delta != 0 {
                term.scroll_viewport(delta as i32);
            }
        }

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
            pty,
            minimap,
            router,
            selection: TextSelection::new(),
            overlay: CommandOverlay::new(),
            pty_mirror: Vec::with_capacity(8192),
            child_exited: false,
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

        // Feed scrollback with null pty (safe, setup_effects not called yet)
        if !scrollback.is_empty() {
            for line in scrollback.lines() {
                let mut buf = line.as_bytes().to_vec();
                buf.extend_from_slice(b"\r\n");
                term.vt_write(&buf);
            }
            term.drain_vt_mirror();
        }

        // Wire effects with -1 fd (harmless)
        term.setup_effects(pty.master_fd(), cw, ch);

        // Restore scroll position
        if let Some((total, _offset, len)) = term.get_scrollbar() {
            let current_bottom = total.saturating_sub(len) as i64;
            let delta = scroll_offset as i64 - current_bottom;
            if delta != 0 {
                term.scroll_viewport(delta as i32);
            }
        }

        term.last_osc_title = title.to_string();

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
            pty,
            minimap,
            router,
            selection: TextSelection::new(),
            overlay: CommandOverlay::new(),
            pty_mirror: Vec::with_capacity(8192),
            child_exited: true,
        })
    }

    pub fn read_pty(&mut self) {
        if self.child_exited {
            return;
        }
        let capture = self.router.capture_buffer();
        match self.pty.read_nonblocking(&mut self.term, capture, Some(&mut self.pty_mirror)) {
            PtyReadResult::Ok => {}
            PtyReadResult::Eof | PtyReadResult::Error => {
                self.child_exited = true;
            }
        }
        if !self.pty_mirror.is_empty() {
            self.minimap.feed(&self.pty_mirror);
            self.pty_mirror.clear();
        }
    }

    pub fn poll_ai(&mut self) {
        self.router.poll_ai_responses(&mut self.term, &self.pty, &mut self.overlay);
        if let Some(vt_data) = self.term.drain_vt_mirror() {
            self.minimap.feed(&vt_data);
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cw: u32, ch: u32) {
        self.term.resize(cols, rows, cw, ch);
        self.pty.resize(cols, rows, cw as i32, ch as i32);
        self.minimap.set_cols(cols);
        let buffer_text = self.term.get_buffer_text(0);
        self.minimap.rebuild_from_text(&buffer_text);
    }

    pub fn title(&self) -> String {
        if !self.term.last_osc_title.is_empty() {
            return self.term.last_osc_title.clone();
        }
        match self.pty.get_foreground_process_name() {
            Some(name) => name,
            None => {
                self.pty.get_cwd()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| "shell".to_string())
            }
        }
    }
}
