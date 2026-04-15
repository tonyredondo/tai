use crate::split::SplitNode;
use crate::terminal::ssh::SshTabInfo;

pub const SIDEBAR_DEFAULT_WIDTH: i32 = 200;
pub const SIDEBAR_MIN_WIDTH: i32 = 120;
pub const SIDEBAR_MAX_WIDTH: i32 = 400;
pub const ROW_HEIGHT: i32 = 56;
pub const SIDEBAR_BUTTON_HEIGHT: i32 = 36;

pub struct Workspace {
    pub name: String,
    pub root: SplitNode,
    pub focused_panel_id: u32,
    pub next_panel_id: u32,
    pub ssh_info: Option<SshTabInfo>,
    pub ssh_password: String,
}

impl Workspace {
    pub fn panel_count(&self) -> usize {
        self.root.panel_count()
    }

    pub fn total_tab_count(&self) -> usize {
        let mut count = 0;
        self.root.for_each_panel(&mut |panel| {
            count += panel.tabs.len();
        });
        count
    }
}

pub struct WorkspaceManager {
    pub workspaces: Vec<Workspace>,
    pub active: usize,
    pub sidebar_visible: bool,
    pub sidebar_scroll: i32,
    pub sidebar_width_px: i32,
    next_workspace_id: u32,
    pub renaming: Option<usize>,
    pub rename_buf: String,
    pub context_menu: Option<(usize, i32, i32)>,
}

impl WorkspaceManager {
    pub fn new(initial: Workspace) -> Self {
        Self {
            workspaces: vec![initial],
            active: 0,
            sidebar_visible: true,
            sidebar_scroll: 0,
            sidebar_width_px: SIDEBAR_DEFAULT_WIDTH,
            next_workspace_id: 1,
            renaming: None,
            rename_buf: String::new(),
            context_menu: None,
        }
    }

    pub fn active(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }

    pub fn sidebar_width(&self) -> i32 {
        if self.sidebar_visible { self.sidebar_width_px } else { 0 }
    }

    pub fn next_name(&mut self) -> String {
        let id = self.next_workspace_id;
        self.next_workspace_id += 1;
        format!("workspace {id}")
    }

    pub fn add(&mut self, ws: Workspace) {
        self.workspaces.push(ws);
        self.active = self.workspaces.len() - 1;
    }

    pub fn remove(&mut self, idx: usize) {
        self.workspaces.remove(idx);
        if idx < self.active {
            self.active -= 1;
        } else if self.active >= self.workspaces.len() {
            self.active = self.workspaces.len().saturating_sub(1);
        }
    }

    pub fn switch_to(&mut self, idx: usize) {
        if idx < self.workspaces.len() {
            self.active = idx;
        }
    }

    pub fn next(&mut self) {
        if !self.workspaces.is_empty() {
            self.active = (self.active + 1) % self.workspaces.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.workspaces.is_empty() {
            self.active = if self.active == 0 {
                self.workspaces.len() - 1
            } else {
                self.active - 1
            };
        }
    }

    pub fn from_restored(
        workspaces: Vec<Workspace>,
        active: usize,
        sidebar_visible: bool,
        sidebar_width_px: i32,
        next_workspace_id: u32,
    ) -> Self {
        Self {
            active: active.min(workspaces.len().saturating_sub(1)),
            workspaces,
            sidebar_visible,
            sidebar_scroll: 0,
            sidebar_width_px: sidebar_width_px.max(SIDEBAR_MIN_WIDTH).min(SIDEBAR_MAX_WIDTH),
            next_workspace_id,
            renaming: None,
            rename_buf: String::new(),
            context_menu: None,
        }
    }
}
