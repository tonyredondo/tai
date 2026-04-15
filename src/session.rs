use crate::config::TaiConfig;
use crate::split::{panel_term_size, PanelRect, Panel, SplitDirection, SplitNode};
use crate::tab::TabSession;
use crate::tab_bar::TabBar;
use crate::terminal::ssh::SshTabInfo;
use crate::workspace::{Workspace, WorkspaceManager};

use async_openai::types::{
    ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessage,
    ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestMessage,
    ChatCompletionRequestToolMessage, ChatCompletionRequestToolMessageContent,
    ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
    ChatCompletionToolType, FunctionCall,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const SESSION_VERSION: u32 = 1;
const MAX_SCROLLBACK_LINES: usize = 5000;

// ---------------------------------------------------------------------------
// Serde DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct SessionWorkspace {
    pub name: String,
    pub tree: SessionNode,
    pub focused_panel_id: u32,
    pub next_panel_id: u32,
    #[serde(default)]
    pub ssh_host: String,
    #[serde(default)]
    pub ssh_port: u16,
    #[serde(default)]
    pub ssh_user: String,
    #[serde(default)]
    pub ssh_password: String,
}

#[derive(Serialize, Deserialize)]
pub struct SessionState {
    pub version: u32,
    pub font_size: i32,
    #[serde(default)]
    pub window_width: i32,
    #[serde(default)]
    pub window_height: i32,
    #[serde(default)]
    pub window_x: i32,
    #[serde(default)]
    pub window_y: i32,
    #[serde(default)]
    pub active_workspace: usize,
    #[serde(default = "default_true")]
    pub sidebar_visible: bool,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width_px: i32,
    #[serde(default)]
    pub workspaces: Vec<SessionWorkspace>,
    // v1 compat fields (kept for deserialization, skipped on v2 serialize)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_panel_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_panel_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<SessionNode>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ssh_host: String,
    #[serde(default)]
    pub ssh_port: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ssh_user: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ssh_password: String,
}

fn default_true() -> bool { true }
fn default_sidebar_width() -> i32 { crate::workspace::SIDEBAR_DEFAULT_WIDTH }

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionNode {
    #[serde(rename = "leaf")]
    Leaf { panel: SessionPanel },
    #[serde(rename = "split")]
    Split {
        direction: String,
        ratio: f32,
        left: Box<SessionNode>,
        right: Box<SessionNode>,
    },
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionPanel {
    pub id: u32,
    pub active_tab: usize,
    pub tabs: Vec<SessionTab>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionTab {
    pub cwd: String,
    pub title: String,
    pub scrollback: String,
    pub scroll_offset: u64,
    pub child_exited: bool,
    pub router: SessionRouter,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub ssh_host: String,
    #[serde(default)]
    pub ssh_port: u16,
    #[serde(default)]
    pub ssh_user: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionRouter {
    pub unified_history: Vec<String>,
    pub command_history: Vec<String>,
    pub auto_execute: bool,
    pub conversation: Vec<SessionConversationMessage>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionConversationMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<SessionToolCall>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

pub fn session_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("tai"))
}

pub fn session_path() -> Option<PathBuf> {
    session_dir().map(|d| d.join("session.json"))
}

fn session_tmp_path() -> Option<PathBuf> {
    session_dir().map(|d| d.join("session.json.tmp"))
}

pub fn sessions_dir() -> Option<PathBuf> {
    session_dir().map(|d| d.join("sessions"))
}

// ---------------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------------

pub fn save(
    wm: &WorkspaceManager,
    font_size: i32,
    window_width: i32,
    window_height: i32,
    window_x: i32,
    window_y: i32,
) -> Result<(), String> {
    let workspaces: Vec<SessionWorkspace> = wm.workspaces.iter().map(|ws| {
        let (ssh_host, ssh_port, ssh_user, ssh_password) = match &ws.ssh_info {
            Some(info) => (info.host.clone(), info.port, info.user.clone(), ws.ssh_password.clone()),
            None => (String::new(), 0, String::new(), String::new()),
        };
        SessionWorkspace {
            name: ws.name.clone(),
            tree: split_node_to_session(&ws.root),
            focused_panel_id: ws.focused_panel_id,
            next_panel_id: ws.next_panel_id,
            ssh_host,
            ssh_port,
            ssh_user,
            ssh_password,
        }
    }).collect();

    let state = SessionState {
        version: SESSION_VERSION,
        font_size,
        window_width,
        window_height,
        window_x,
        window_y,
        active_workspace: wm.active,
        sidebar_visible: wm.sidebar_visible,
        sidebar_width_px: wm.sidebar_width_px,
        workspaces,
        focused_panel_id: None,
        next_panel_id: None,
        tree: None,
        ssh_host: String::new(),
        ssh_port: 0,
        ssh_user: String::new(),
        ssh_password: String::new(),
    };

    let json = serde_json::to_string(&state).map_err(|e| format!("serialize: {e}"))?;

    let dir = session_dir().ok_or("no config dir")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create dir: {e}"))?;

    let tmp = session_tmp_path().ok_or("no config dir")?;
    let dst = session_path().ok_or("no config dir")?;

    std::fs::write(&tmp, json.as_bytes()).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, &dst).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

fn split_node_to_session(node: &SplitNode) -> SessionNode {
    match node {
        SplitNode::Leaf(panel) => {
            let tabs: Vec<SessionTab> = panel
                .tabs
                .iter()
                .map(|tab| tab_to_session(tab))
                .collect();
            SessionNode::Leaf {
                panel: SessionPanel {
                    id: panel.id,
                    active_tab: panel.active_tab,
                    tabs,
                },
            }
        }
        SplitNode::Split {
            direction,
            ratio,
            left,
            right,
        } => {
            let dir_str = match direction {
                SplitDirection::Horizontal => "horizontal",
                SplitDirection::Vertical => "vertical",
            };
            SessionNode::Split {
                direction: dir_str.to_string(),
                ratio: *ratio,
                left: Box::new(split_node_to_session(left)),
                right: Box::new(split_node_to_session(right)),
            }
        }
    }
}

fn is_vt_blank_line(line: &[u8]) -> bool {
    let mut i = 0;
    while i < line.len() {
        match line[i] {
            0x1b => {
                i += 1;
                if i < line.len() && line[i] == b'[' {
                    i += 1;
                    while i < line.len() && !(line[i] >= b'@' && line[i] <= b'~') {
                        i += 1;
                    }
                    if i < line.len() {
                        i += 1;
                    }
                } else if i < line.len() {
                    i += 1;
                }
            }
            b' ' | b'\t' | b'\r' => i += 1,
            _ => return false,
        }
    }
    true
}

fn strip_trailing_prompt(scrollback: &mut String) {
    let mut search_start = scrollback.len().saturating_sub(4000);
    while search_start > 0 && !scrollback.is_char_boundary(search_start) {
        search_start -= 1;
    }
    let tail = scrollback[search_start..].as_bytes();

    let mut line_boundaries: Vec<usize> = Vec::new();
    for (i, &b) in tail.iter().enumerate() {
        if b == b'\n' {
            line_boundaries.push(i);
        }
    }

    let mut trunc_pos = None;
    for w in line_boundaries.windows(2).rev() {
        let line_start = w[0] + 1;
        let line_end = w[1];
        let line = &tail[line_start..line_end];
        let line = if line.last() == Some(&b'\r') {
            &line[..line.len() - 1]
        } else {
            line
        };
        if is_vt_blank_line(line) {
            trunc_pos = Some(search_start + line_start);
            break;
        }
    }

    if trunc_pos.is_none() && !line_boundaries.is_empty() {
        let first_nl = line_boundaries[0];
        let first_line = &tail[..first_nl];
        let first_line = if first_line.last() == Some(&b'\r') {
            &first_line[..first_line.len() - 1]
        } else {
            first_line
        };
        if is_vt_blank_line(first_line) {
            trunc_pos = Some(search_start);
        }
    }

    match trunc_pos {
        Some(0) => scrollback.clear(),
        Some(pos) => scrollback.truncate(pos),
        None => scrollback.clear(),
    }
}

fn tab_to_session(tab: &TabSession) -> SessionTab {
    let cwd = if tab.child_exited {
        String::new()
    } else {
        tab.backend
            .get_cwd()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    };

    // If we injected scrollback from a session load, use the original
    // scrollback text for saving to prevent accumulation/corruption
    // across save/load cycles.
    let mut scrollback = if let Some(ref orig) = tab.original_scrollback {
        orig.clone()
    } else {
        tab.term.get_buffer_vt()
    };

    if !tab.child_exited && tab.original_scrollback.is_none() {
        strip_trailing_prompt(&mut scrollback);
    }

    let line_count = scrollback.lines().count();
    if line_count > MAX_SCROLLBACK_LINES {
        let skip = line_count - MAX_SCROLLBACK_LINES;
        if let Some(pos) = scrollback
            .char_indices()
            .filter(|(_, c)| *c == '\n')
            .nth(skip - 1)
            .map(|(i, _)| i + 1)
        {
            scrollback = scrollback[pos..].to_string();
        }
    }

    let scroll_offset = tab
        .term
        .get_scrollbar()
        .map(|(_, offset, _)| offset)
        .unwrap_or(0);

    let conversation: Vec<SessionConversationMessage> = tab
        .router
        .conversation_messages()
        .iter()
        .filter_map(msg_to_session)
        .collect();

    let (kind, ssh_host, ssh_port, ssh_user) = if let Some(ref info) = tab.ssh_info {
        ("ssh".to_string(), info.host.clone(), info.port, info.user.clone())
    } else {
        (String::new(), String::new(), 0, String::new())
    };

    SessionTab {
        cwd,
        title: (*tab.term.last_osc_title).clone(),
        scrollback,
        scroll_offset,
        child_exited: tab.child_exited,
        router: SessionRouter {
            unified_history: tab.router.unified_history().clone(),
            command_history: tab.router.command_history().clone(),
            auto_execute: tab.router.auto_execute(),
            conversation,
        },
        kind,
        ssh_host,
        ssh_port,
        ssh_user,
    }
}

fn msg_to_session(msg: &ChatCompletionRequestMessage) -> Option<SessionConversationMessage> {
    match msg {
        ChatCompletionRequestMessage::User(u) => {
            let content = match &u.content {
                ChatCompletionRequestUserMessageContent::Text(t) => t.clone(),
                _ => return None,
            };
            Some(SessionConversationMessage {
                role: "user".into(),
                content: Some(content),
                tool_call_id: None,
                tool_calls: None,
            })
        }
        ChatCompletionRequestMessage::Assistant(a) => {
            let content = a.content.as_ref().and_then(|c| match c {
                ChatCompletionRequestAssistantMessageContent::Text(t) => Some(t.clone()),
                _ => None,
            });
            let tool_calls = a.tool_calls.as_ref().map(|tcs| {
                tcs.iter()
                    .map(|tc| SessionToolCall {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        arguments: tc.function.arguments.clone(),
                    })
                    .collect()
            });
            Some(SessionConversationMessage {
                role: "assistant".into(),
                content,
                tool_call_id: None,
                tool_calls,
            })
        }
        ChatCompletionRequestMessage::Tool(t) => {
            let content = match &t.content {
                ChatCompletionRequestToolMessageContent::Text(text) => text.clone(),
                _ => return None,
            };
            Some(SessionConversationMessage {
                role: "tool".into(),
                content: Some(content),
                tool_call_id: Some(t.tool_call_id.clone()),
                tool_calls: None,
            })
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

pub fn load() -> Result<Option<SessionState>, String> {
    let path = match session_path() {
        Some(p) => p,
        None => return Ok(None),
    };

    if !path.exists() {
        if let Some(tmp) = session_tmp_path() {
            if tmp.exists() {
                return load_from_path(&tmp);
            }
        }
        return Ok(None);
    }

    match load_from_path(&path) {
        Ok(Some(s)) => Ok(Some(s)),
        Ok(None) => Ok(None),
        Err(_) => {
            if let Some(tmp) = session_tmp_path() {
                if tmp.exists() {
                    return load_from_path(&tmp);
                }
            }
            Ok(None)
        }
    }
}

fn load_from_path(path: &std::path::Path) -> Result<Option<SessionState>, String> {
    let data = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let state: SessionState =
        serde_json::from_str(&data).map_err(|e| format!("parse: {e}"))?;
    if state.version != SESSION_VERSION {
        return Ok(None);
    }
    Ok(Some(state))
}

// ---------------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------------

pub fn restore(
    state: SessionState,
    config: &TaiConfig,
    scr_w: i32,
    scr_h: i32,
    status_bar_height: i32,
    sidebar_w: i32,
    pad: i32,
    minimap_width: i32,
    cw: i32,
    ch: i32,
) -> Result<(WorkspaceManager, i32), String> {
    // Normalize: if v1 format (no workspaces vec), migrate single tree to one workspace
    let session_workspaces = if state.workspaces.is_empty() {
        if let Some(tree) = state.tree {
            vec![SessionWorkspace {
                name: "default".to_string(),
                tree,
                focused_panel_id: state.focused_panel_id.unwrap_or(0),
                next_panel_id: state.next_panel_id.unwrap_or(1),
                ssh_host: state.ssh_host,
                ssh_port: state.ssh_port,
                ssh_user: state.ssh_user,
                ssh_password: state.ssh_password,
            }]
        } else {
            return Err("No workspace data found".into());
        }
    } else {
        state.workspaces
    };

    let mut workspaces: Vec<Workspace> = Vec::new();

    for sw in &session_workspaces {
        match restore_single_workspace(sw, config, scr_w, scr_h, status_bar_height, sidebar_w, pad, minimap_width, cw, ch) {
            Ok(ws) => workspaces.push(ws),
            Err(e) => eprintln!("[TAI] Failed to restore workspace '{}': {e}", sw.name),
        }
    }

    if workspaces.is_empty() {
        return Err("All workspaces failed to restore".into());
    }

    let next_id = workspaces.len() as u32 + 1;
    let wm = WorkspaceManager::from_restored(
        workspaces,
        state.active_workspace,
        state.sidebar_visible,
        state.sidebar_width_px,
        next_id,
    );

    Ok((wm, state.font_size))
}

fn restore_single_workspace(
    sw: &SessionWorkspace,
    config: &TaiConfig,
    scr_w: i32,
    scr_h: i32,
    status_bar_height: i32,
    sidebar_w: i32,
    pad: i32,
    minimap_width: i32,
    cw: i32,
    ch: i32,
) -> Result<Workspace, String> {
    let initial_rect = PanelRect {
        x: sidebar_w,
        y: 0,
        w: scr_w - sidebar_w,
        h: scr_h - status_bar_height,
    };

    let mut skeleton = build_skeleton(&sw.tree, ch);
    skeleton.layout(initial_rect);

    let mut panel_data: Vec<(u32, PanelRect, i32, Vec<SessionTab>, usize)> = Vec::new();
    skeleton.for_each_panel(&mut |panel| {
        panel_data.push((
            panel.id,
            panel.rect,
            panel.tab_bar.height,
            Vec::new(),
            0,
        ));
    });

    let saved_panels = collect_session_panels(&sw.tree);
    for pd in &mut panel_data {
        if let Some(sp) = saved_panels.iter().find(|sp| sp.id == pd.0) {
            pd.3 = sp.tabs.clone();
            pd.4 = sp.active_tab;
        }
    }

    for pd in &panel_data {
        let (panel_id, rect, tab_bar_h, saved_tabs, saved_active) = (pd.0, pd.1, pd.2, &pd.3, pd.4);
        let (cols, rows) = panel_term_size(&rect, pad, minimap_width, tab_bar_h, cw, ch);

        let mut live_tabs: Vec<TabSession> = Vec::new();
        for st in saved_tabs {
            match restore_tab(st, config, cols, rows, cw, ch) {
                Ok(tab) => live_tabs.push(tab),
                Err(e) => eprintln!("[TAI] Failed to restore tab: {e}"),
            }
        }

        if live_tabs.is_empty() {
            match TabSession::new(config, cols, rows, cw, ch) {
                Ok(tab) => live_tabs.push(tab),
                Err(e) => {
                    eprintln!("[TAI] Failed to create fallback tab: {e}");
                    continue;
                }
            }
        }

        let clamped_active = saved_active.min(live_tabs.len().saturating_sub(1));

        if let Some(panel) = skeleton.panel_by_id_mut(panel_id) {
            panel.tabs = live_tabs;
            panel.active_tab = clamped_active;
        }
    }

    let mut focused = sw.focused_panel_id;
    if skeleton.panel_by_id(focused).is_none() {
        let leaves = skeleton.collect_leaves();
        focused = leaves.first().copied().unwrap_or(0);
    }

    let ssh_info = if !sw.ssh_host.is_empty() && !sw.ssh_user.is_empty() {
        Some(SshTabInfo {
            host: sw.ssh_host.clone(),
            port: sw.ssh_port,
            user: sw.ssh_user.clone(),
        })
    } else {
        None
    };

    Ok(Workspace {
        name: sw.name.clone(),
        root: skeleton,
        focused_panel_id: focused,
        next_panel_id: sw.next_panel_id,
        ssh_info,
        ssh_password: sw.ssh_password.clone(),
    })
}

fn build_skeleton(node: &SessionNode, ch: i32) -> SplitNode {
    match node {
        SessionNode::Leaf { panel: sp } => {
            let panel = Panel {
                id: sp.id,
                tabs: Vec::new(),
                active_tab: 0,
                tab_bar: TabBar::new(ch),
                rect: PanelRect { x: 0, y: 0, w: 0, h: 0 },
            };
            SplitNode::Leaf(panel)
        }
        SessionNode::Split {
            direction,
            ratio,
            left,
            right,
        } => {
            let dir = if direction == "vertical" {
                SplitDirection::Vertical
            } else {
                SplitDirection::Horizontal
            };
            SplitNode::Split {
                direction: dir,
                ratio: *ratio,
                left: Box::new(build_skeleton(left, ch)),
                right: Box::new(build_skeleton(right, ch)),
            }
        }
    }
}

fn collect_session_panels(node: &SessionNode) -> Vec<SessionPanel> {
    match node {
        SessionNode::Leaf { panel } => vec![panel.clone()],
        SessionNode::Split { left, right, .. } => {
            let mut v = collect_session_panels(left);
            v.extend(collect_session_panels(right));
            v
        }
    }
}

fn restore_tab(
    st: &SessionTab,
    config: &TaiConfig,
    cols: u16,
    rows: u16,
    cw: i32,
    ch: i32,
) -> Result<TabSession, String> {
    if st.kind == "ssh" {
        let title = format!("{}@{}:{} (disconnected)", st.ssh_user, st.ssh_host, st.ssh_port);
        let mut tab = TabSession::new_dead(config, &title, &st.scrollback, st.scroll_offset, cols, rows, cw, ch, &st.router)?;
        tab.ssh_info = Some(crate::terminal::ssh::SshTabInfo {
            host: st.ssh_host.clone(),
            port: st.ssh_port,
            user: st.ssh_user.clone(),
        });
        return Ok(tab);
    }

    if st.child_exited {
        return TabSession::new_dead(config, &st.title, &st.scrollback, st.scroll_offset, cols, rows, cw, ch, &st.router);
    }
    let cwd = PathBuf::from(&st.cwd);
    let result = if !st.cwd.is_empty() && cwd.is_dir() {
        TabSession::new_in_dir(config, cwd, cols, rows, cw, ch, &st.scrollback, st.scroll_offset, &st.router)
    } else {
        Err("invalid cwd".into())
    };

    match result {
        Ok(tab) => Ok(tab),
        Err(_) => {
            TabSession::new_in_dir(
                config,
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
                cols, rows, cw, ch,
                &st.scrollback, st.scroll_offset, &st.router,
            )
            .or_else(|_| {
                let mut tab = TabSession::new(config, cols, rows, cw, ch)?;
                let messages = session_messages_to_api(&st.router.conversation);
                tab.router.restore_history(
                    st.router.unified_history.clone(),
                    st.router.command_history.clone(),
                    st.router.auto_execute,
                );
                tab.router.restore_conversation(messages);
                Ok(tab)
            })
        }
    }
}

pub fn session_messages_to_api(
    msgs: &[SessionConversationMessage],
) -> Vec<ChatCompletionRequestMessage> {
    msgs.iter().filter_map(session_msg_to_api).collect()
}

fn session_msg_to_api(msg: &SessionConversationMessage) -> Option<ChatCompletionRequestMessage> {
    match msg.role.as_str() {
        "user" => {
            let content = msg.content.as_ref()?.clone();
            Some(ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text(content),
                    name: None,
                },
            ))
        }
        "assistant" => {
            let content = msg.content.as_ref().map(|t| {
                ChatCompletionRequestAssistantMessageContent::Text(t.clone())
            });
            let tool_calls = msg.tool_calls.as_ref().map(|tcs| {
                tcs.iter()
                    .map(|tc| ChatCompletionMessageToolCall {
                        id: tc.id.clone(),
                        r#type: ChatCompletionToolType::Function,
                        function: FunctionCall {
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        },
                    })
                    .collect()
            });
            Some(ChatCompletionRequestMessage::Assistant(
                ChatCompletionRequestAssistantMessage {
                    content,
                    name: None,
                    tool_calls,
                    refusal: None,
                    audio: None,
                    #[allow(deprecated)]
                    function_call: None,
                },
            ))
        }
        "tool" => {
            let content = msg.content.as_ref()?.clone();
            let tool_call_id = msg.tool_call_id.as_ref()?.clone();
            Some(ChatCompletionRequestMessage::Tool(
                ChatCompletionRequestToolMessage {
                    tool_call_id,
                    content: ChatCompletionRequestToolMessageContent::Text(content),
                },
            ))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Export / Import / Reset / List
// ---------------------------------------------------------------------------

pub fn export_session(name: &str) -> Result<(), String> {
    let src = session_path().ok_or("no config dir")?;
    if !src.exists() {
        return Err("No session to export. Run TAI first.".into());
    }
    let dir = sessions_dir().ok_or("no config dir")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create sessions dir: {e}"))?;
    let dst = dir.join(format!("{name}.json"));
    std::fs::copy(&src, &dst).map_err(|e| format!("copy: {e}"))?;
    Ok(())
}

pub fn import_session(name: &str) -> Result<Option<SessionState>, String> {
    let dir = sessions_dir().ok_or("no config dir")?;
    let path = dir.join(format!("{name}.json"));
    if !path.exists() {
        return Err(format!("Session '{name}' not found"));
    }
    load_from_path(&path)
}

pub fn reset_session() -> Result<(), String> {
    if let Some(path) = session_path() {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("remove: {e}"))?;
        }
    }
    if let Some(tmp) = session_tmp_path() {
        if tmp.exists() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    Ok(())
}

pub fn delete_named_session(name: &str) -> Result<(), String> {
    let dir = sessions_dir().ok_or("no config dir")?;
    let path = dir.join(format!("{name}.json"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Delete failed: {e}"))
    } else {
        Err(format!("Session '{name}' not found"))
    }
}

pub fn list_sessions() -> Vec<String> {
    let dir = match sessions_dir() {
        Some(d) => d,
        None => return Vec::new(),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").map(|s| s.to_string())
        })
        .collect();
    names.sort();
    names
}
