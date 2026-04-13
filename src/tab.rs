use crate::ai::bridge::AiBridge;
use crate::config::TaiConfig;
use crate::minimap::Minimap;
use crate::overlay::CommandOverlay;
use crate::router::InputRouter;
use crate::selection::TextSelection;
use crate::terminal::engine::{PtyReadResult, Terminal};
use crate::terminal::pty::Pty;

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
