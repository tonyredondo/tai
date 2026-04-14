use crate::ai::bridge::{AiBridge, AiRequest, AiResponse};
use crate::ai::context::ContextBuilder;
use crate::ai::conversation::ConversationHistory;
use crate::config::TaiConfig;
use crate::overlay::CommandOverlay;
use crate::terminal::engine::Terminal;
use crate::terminal::pty::Pty;
use async_openai::types::ChatCompletionMessageToolCall;
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    Shell,
    AiPrompt,
    AiStreaming,
    CommandConfirm,
}

struct CommandCapture {
    start_marker: String,
    end_marker: String,
    active: bool,
    timeout: Instant,
    tool_call_id: String,
    raw_output: Vec<u8>,
}

struct QueuedToolCall {
    id: String,
    name: String,
    command: String,
    explanation: String,
}

pub struct InputRouter {
    mode: InputMode,
    ai_input_buffer: String,
    unified_history: Vec<String>,
    history_index: Option<usize>,
    history_draft: String,
    pending_command: Option<PendingCommand>,
    command_history: Vec<String>,
    current_line_buffer: String,
    ai_bridge: Option<AiBridge>,
    conversation: ConversationHistory,
    context_builder: ContextBuilder,
    config: TaiConfig,
    ai_text_buffer: String,
    command_capture: Option<CommandCapture>,
    tool_call_queue: Vec<QueuedToolCall>,
    pending_tool_calls: Vec<(String, String, String)>,
    has_pending_dispatch: bool,
    ai_first_token: bool,
    ai_after_command: bool,
}

pub struct PendingCommand {
    pub command: String,
    pub explanation: String,
    pub tool_call_id: String,
}

impl InputRouter {
    pub fn new(config: &TaiConfig, ai_bridge: Option<AiBridge>) -> Self {
        InputRouter {
            mode: InputMode::Shell,
            ai_input_buffer: String::new(),
            unified_history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            pending_command: None,
            command_history: Vec::new(),
            current_line_buffer: String::new(),
            ai_bridge,
            conversation: ConversationHistory::new(config.ai.max_history),
            context_builder: ContextBuilder::new(config.ai.max_context_lines),
            config: config.clone(),
            ai_text_buffer: String::new(),
            command_capture: None,
            tool_call_queue: Vec::new(),
            pending_tool_calls: Vec::new(),
            has_pending_dispatch: false,
            ai_first_token: false,
            ai_after_command: false,
        }
    }

    pub fn mode(&self) -> InputMode {
        self.mode
    }

    pub fn auto_execute(&self) -> bool {
        self.config.ai.auto_execute
    }

    pub fn toggle_auto_execute(&mut self) {
        self.config.ai.auto_execute = !self.config.ai.auto_execute;
    }

    pub fn capture_buffer(&mut self) -> Option<&mut Vec<u8>> {
        self.command_capture
            .as_mut()
            .filter(|c| c.active)
            .map(|c| &mut c.raw_output)
    }

    pub fn ai_input_buffer(&self) -> &str {
        &self.ai_input_buffer
    }

    pub fn toggle_ai_mode(&mut self) {
        match self.mode {
            InputMode::Shell => {
                if self.ai_bridge.is_some() {
                    self.mode = InputMode::AiPrompt;
                    self.ai_input_buffer.clear();
                    self.history_index = None;
                    self.history_draft.clear();
                }
            }
            InputMode::AiPrompt => {
                self.mode = InputMode::Shell;
                self.ai_input_buffer.clear();
                self.history_index = None;
            }
            _ => {}
        }
    }

    pub fn handle_ai_prompt_char(&mut self, c: char) {
        if self.mode == InputMode::AiPrompt {
            self.ai_input_buffer.push(c);
        }
    }

    pub fn handle_ai_prompt_backspace(&mut self) {
        if self.mode == InputMode::AiPrompt {
            self.ai_input_buffer.pop();
        }
    }

    pub fn handle_ai_prompt_submit(&mut self, terminal: &mut Terminal, pty: &Pty) {
        if self.mode != InputMode::AiPrompt || self.ai_input_buffer.is_empty() {
            return;
        }

        let query = self.ai_input_buffer.clone();

        if query.trim() == "/clear" {
            self.conversation.clear();
            self.ai_input_buffer.clear();
            self.history_index = None;
            self.mode = InputMode::Shell;
            return;
        }

        self.push_unified_history(&query);
        self.history_index = None;

        let prompt_display = format!(
            "\r\x1b[2K\x1b[1;35m❯ {}\x1b[0m",
            query.replace('\n', "\r\n  ")
        );
        terminal.vt_write(prompt_display.as_bytes());

        let buffer_text = terminal.get_buffer_text(self.config.ai.max_context_lines);
        let cwd = pty.get_cwd().unwrap_or_else(|| PathBuf::from("~"));
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());

        let system_msg = ContextBuilder::build_system_message(os, arch, &shell, &cwd);
        let user_msg = self.context_builder.build_user_message(
            &query,
            &buffer_text,
            &self.command_history,
        );

        self.conversation.push_message(user_msg);

        let messages = self.conversation.build_messages(system_msg);

        if let Some(ref bridge) = self.ai_bridge {
            bridge.send(AiRequest::Chat { messages });
        }

        self.ai_input_buffer.clear();
        self.ai_text_buffer.clear();
        self.ai_first_token = true;
        self.mode = InputMode::AiStreaming;
    }

    pub fn handle_ai_prompt_history_up(&mut self) {
        if self.mode != InputMode::AiPrompt || self.unified_history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.history_draft = self.ai_input_buffer.clone();
                let idx = self.unified_history.len() - 1;
                self.history_index = Some(idx);
                self.ai_input_buffer = self.unified_history[idx].clone();
            }
            Some(idx) if idx > 0 => {
                let new_idx = idx - 1;
                self.history_index = Some(new_idx);
                self.ai_input_buffer = self.unified_history[new_idx].clone();
            }
            _ => {}
        }
    }

    pub fn handle_ai_prompt_history_down(&mut self) {
        if self.mode != InputMode::AiPrompt {
            return;
        }
        match self.history_index {
            Some(idx) => {
                if idx + 1 < self.unified_history.len() {
                    let new_idx = idx + 1;
                    self.history_index = Some(new_idx);
                    self.ai_input_buffer = self.unified_history[new_idx].clone();
                } else {
                    self.history_index = None;
                    self.ai_input_buffer = self.history_draft.clone();
                    self.history_draft.clear();
                }
            }
            None => {}
        }
    }

    pub fn handle_ai_prompt_cancel(&mut self) {
        if self.mode == InputMode::AiPrompt {
            self.mode = InputMode::Shell;
            self.ai_input_buffer.clear();
        }
    }

    pub fn poll_ai_responses(&mut self, terminal: &mut Terminal, pty: &Pty, overlay: &mut CommandOverlay) {
        let should_finish_timeout = self.command_capture.as_ref()
            .is_some_and(|c| c.active && c.timeout.elapsed().as_secs() > 30);

        if should_finish_timeout {
            self.finish_command_capture(terminal, pty);
        }

        let has_end_marker = self.command_capture.as_ref().is_some_and(|c| {
            c.active && {
                let raw = String::from_utf8_lossy(&c.raw_output);
                let esc_end = format!("\x1b[8m{}", c.end_marker);
                raw.contains(&esc_end)
            }
        });

        if has_end_marker {
            if let Some(ref c) = self.command_capture {
                eprintln!("[TAI] End marker found for tool_call_id={}", c.tool_call_id);
            }
            self.finish_command_capture(terminal, pty);
        }

        if self.has_pending_dispatch && self.command_capture.is_none() {
            self.has_pending_dispatch = false;
            if !self.tool_call_queue.is_empty() {
                eprintln!("[TAI] Dispatching next queued tool call ({} remaining)", self.tool_call_queue.len());
                self.dispatch_next_tool_call(terminal, pty, overlay);
                return;
            }
            eprintln!("[TAI] Queue empty, sending AI request (conversation has {} msgs)", self.conversation.len());
            if let Some(ref bridge) = self.ai_bridge {
                let cwd = pty.get_cwd().unwrap_or_else(|| PathBuf::from("~"));
                let os = std::env::consts::OS;
                let arch = std::env::consts::ARCH;
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
                let system_msg = ContextBuilder::build_system_message(os, arch, &shell, &cwd);
                let messages = self.conversation.build_messages(system_msg);
                bridge.send(AiRequest::Chat { messages });
            }
        }

        let responses: Vec<_> = match self.ai_bridge {
            Some(ref b) => {
                let mut v = Vec::new();
                while let Some(r) = b.try_recv() {
                    v.push(r);
                }
                v
            }
            None => return,
        };

        for response in responses {
            match response {
                AiResponse::Token(token) => {
                    if self.ai_first_token {
                        if self.ai_after_command {
                            terminal.vt_write(b"\r\x1b[2K\x1b[A\r\x1b[2K");
                            self.ai_after_command = false;
                        } else {
                            terminal.vt_write(b"\r\n");
                        }
                        terminal.vt_write(b"\x1b[36m\x1b[1m TAI:\x1b[22m ");
                        self.ai_first_token = false;
                    }
                    self.ai_text_buffer.push_str(&token);
                    let display_token = token.replace('\n', "\r\n");
                    let colored = format!("\x1b[36m{}\x1b[0m", display_token);
                    terminal.vt_write(colored.as_bytes());
                }
                AiResponse::ToolCall { id, name, arguments } => {
                    self.pending_tool_calls.push((id, name, arguments));
                }
                AiResponse::Done => {
                    if self.command_capture.is_some() {
                        continue;
                    }

                    if !self.pending_tool_calls.is_empty() {
                        let batch = std::mem::take(&mut self.pending_tool_calls);
                        eprintln!("[TAI] Done: committing {} tool calls as single assistant message", batch.len());

                        if self.ai_after_command {
                            terminal.vt_write(b"\r\x1b[2K\x1b[A\r\x1b[2K");
                            self.ai_after_command = false;
                        }

                        let api_tool_calls: Vec<ChatCompletionMessageToolCall> = batch
                            .iter()
                            .map(|(id, name, args)| ChatCompletionMessageToolCall {
                                id: id.clone(),
                                r#type: async_openai::types::ChatCompletionToolType::Function,
                                function: async_openai::types::FunctionCall {
                                    name: name.clone(),
                                    arguments: args.clone(),
                                },
                            })
                            .collect();
                        self.conversation.push_assistant_tool_call(api_tool_calls);

                        for (id, name, arguments) in batch {
                            if name == "run_command" {
                                if let Ok(args) = serde_json::from_str::<serde_json::Value>(&arguments) {
                                    let command = args["command"].as_str().unwrap_or("").to_string();
                                    let explanation = args["explanation"].as_str().unwrap_or("").to_string();
                                    self.tool_call_queue.push(QueuedToolCall {
                                        id,
                                        name,
                                        command,
                                        explanation,
                                    });
                                }
                            }
                        }

                        self.dispatch_next_tool_call(terminal, pty, overlay);
                        continue;
                    }

                    if self.ai_after_command {
                        terminal.vt_write(b"\r\x1b[2K\x1b[A\r\x1b[2K");
                        self.ai_after_command = false;
                    }
                    if !self.ai_text_buffer.is_empty() {
                        self.conversation.push_assistant(&self.ai_text_buffer);
                        self.ai_text_buffer.clear();
                    }
                    if self.mode == InputMode::AiStreaming {
                        terminal.vt_write(b"\x1b[0m");
                        pty.write(b"\n");
                        self.mode = InputMode::Shell;
                    }
                }
                AiResponse::Error(err) => {
                    self.pending_tool_calls.clear();
                    let msg = format!("\r\n\x1b[31mTAI Error: {}\x1b[0m\r\n", err);
                    terminal.vt_write(msg.as_bytes());
                    self.conversation.remove_trailing_orphans();
                    pty.write(b"\n");
                    self.mode = InputMode::Shell;
                }
            }
        }
    }

    pub fn handle_command_confirm_enter(
        &mut self,
        terminal: &mut Terminal,
        pty: &Pty,
        overlay: &mut CommandOverlay,
    ) {
        if let Some(pending) = self.pending_command.take() {
            overlay.hide();
            self.execute_command(&pending.command, &pending.tool_call_id, terminal, pty);
        }
    }

    pub fn handle_command_confirm_cancel(
        &mut self,
        pty: &Pty,
        overlay: &mut CommandOverlay,
    ) {
        if let Some(pending) = self.pending_command.take() {
            overlay.hide();
            self.conversation.push_tool_result(&pending.tool_call_id, "User cancelled the command.");
            self.tool_call_queue.clear();
            self.pending_tool_calls.clear();
            pty.write(b"\n");
            self.mode = InputMode::Shell;
        }
    }

    pub fn handle_command_confirm_edit(
        &mut self,
        pty: &Pty,
        overlay: &mut CommandOverlay,
    ) {
        if let Some(pending) = self.pending_command.take() {
            overlay.hide();
            pty.write(pending.command.as_bytes());
            self.conversation.push_tool_result(&pending.tool_call_id, "User chose to edit the command manually.");
            self.tool_call_queue.clear();
            self.pending_tool_calls.clear();
            self.mode = InputMode::Shell;
        }
    }

    pub fn track_shell_char(&mut self, c: char) {
        if self.mode == InputMode::Shell {
            if c == '\n' || c == '\r' {
                if !self.current_line_buffer.is_empty() {
                    self.command_history.push(self.current_line_buffer.clone());
                    if self.command_history.len() > 100 {
                        self.command_history.remove(0);
                    }
                    self.push_unified_history(&self.current_line_buffer.clone());
                    self.current_line_buffer.clear();
                }
            } else if c == '\x7f' || c == '\x08' {
                self.current_line_buffer.pop();
            } else if c.is_ascii_graphic() || c == ' ' {
                self.current_line_buffer.push(c);
            }
        }
    }

    fn push_unified_history(&mut self, entry: &str) {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.unified_history.last().map_or(true, |last| last != trimmed) {
            self.unified_history.push(trimmed.to_string());
        }
        if self.unified_history.len() > 200 {
            self.unified_history.remove(0);
        }
    }

    pub fn ai_available(&self) -> bool {
        self.ai_bridge.is_some()
    }

    pub fn unified_history(&self) -> &Vec<String> {
        &self.unified_history
    }

    pub fn command_history(&self) -> &Vec<String> {
        &self.command_history
    }

    pub fn conversation_messages(&self) -> &std::collections::VecDeque<async_openai::types::ChatCompletionRequestMessage> {
        self.conversation.messages()
    }

    pub fn restore_history(&mut self, unified: Vec<String>, commands: Vec<String>, auto_exec: bool) {
        self.unified_history = unified;
        self.command_history = commands;
        self.config.ai.auto_execute = auto_exec;
    }

    pub fn restore_conversation(&mut self, messages: Vec<async_openai::types::ChatCompletionRequestMessage>) {
        self.conversation.push_restored(messages);
    }

    fn dispatch_next_tool_call(
        &mut self,
        terminal: &mut Terminal,
        pty: &Pty,
        overlay: &mut CommandOverlay,
    ) {
        if self.tool_call_queue.is_empty() {
            return;
        }

        let tc = self.tool_call_queue.remove(0);

        if self.config.ai.auto_execute {
            self.execute_command(&tc.command, &tc.id, terminal, pty);
        } else {
            self.pending_command = Some(PendingCommand {
                command: tc.command,
                explanation: tc.explanation,
                tool_call_id: tc.id,
            });
            overlay.show(
                &self.pending_command.as_ref().unwrap().command,
                &self.pending_command.as_ref().unwrap().explanation,
            );
            self.mode = InputMode::CommandConfirm;
        }
    }

    fn execute_command(
        &mut self,
        command: &str,
        tool_call_id: &str,
        terminal: &mut Terminal,
        pty: &Pty,
    ) {
        let uuid = Uuid::new_v4().to_string()[..8].to_string();
        let start_marker = format!("TAI_S_{uuid}");
        let end_marker = format!("TAI_E_{uuid}:");

        let cmd_display = format!("\r\n\x1b[1;33m$ {}\x1b[0m", command);
        terminal.vt_write(cmd_display.as_bytes());

        let script = format!(
            "printf '\\e[8m{start_marker}\\n\\e[28m'\n\
             {command}\n\
             __tai_ec=$?\n\
             printf '\\e[8m'\n\
             echo '{end_marker}'\"$__tai_ec\"\n\
             printf '\\e[28m'\n\
             rm -f /tmp/tai_cmd_{uuid}\n"
        );
        let tmp_path = format!("/tmp/tai_cmd_{uuid}");
        let _ = std::fs::write(&tmp_path, &script);

        self.command_capture = Some(CommandCapture {
            start_marker,
            end_marker,
            active: true,
            timeout: Instant::now(),
            tool_call_id: tool_call_id.to_string(),
            raw_output: Vec::new(),
        });

        let source_cmd = format!(" . {}\n", tmp_path);
        pty.set_echo(false);
        pty.write(source_cmd.as_bytes());
        self.mode = InputMode::AiStreaming;
    }

    fn finish_command_capture(
        &mut self,
        _terminal: &mut Terminal,
        pty: &Pty,
    ) {
        let capture = match self.command_capture.take() {
            Some(c) => c,
            None => return,
        };

        pty.set_echo(true);

        let raw = String::from_utf8_lossy(&capture.raw_output);
        let mut output = String::new();
        let mut exit_code = -1i32;

        let esc_start = format!("\x1b[8m{}", capture.start_marker);
        let esc_end = format!("\x1b[8m{}", capture.end_marker);

        if let Some(start_pos) = raw.find(&esc_start) {
            let content_start = start_pos + esc_start.len();
            let after_start = &raw[content_start..];
            if let Some(end_pos) = after_start.find(&esc_end) {
                let raw_between = after_start[..end_pos].to_string();
                output = Self::strip_ansi(&raw_between).trim().to_string();
                let after_end = &after_start[end_pos + esc_end.len()..];
                let code_text = Self::strip_ansi(
                    &after_end.lines().next().unwrap_or("").to_string(),
                );
                exit_code = code_text.trim().parse().unwrap_or(-1);
            }
        }

        let truncated = if output.len() > 4000 {
            let cut = &output[..4000];
            format!("{}\n... [truncated, {} bytes total]", cut, output.len())
        } else {
            output.clone()
        };

        let result = format!(
            "Command output (exit code: {exit_code}):\n{truncated}"
        );

        eprintln!("[TAI] finish_command_capture: pushing tool_result for {} (conversation was {} msgs)", capture.tool_call_id, self.conversation.len());
        self.conversation
            .push_tool_result(&capture.tool_call_id, &result);
        eprintln!("[TAI] finish_command_capture: conversation now {} msgs, queue={}", self.conversation.len(), self.tool_call_queue.len());

        self.ai_first_token = true;
        self.ai_after_command = true;
        self.mode = InputMode::AiStreaming;

        self.has_pending_dispatch = true;
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                match chars.peek() {
                    Some('[') => {
                        chars.next();
                        while let Some(&p) = chars.peek() {
                            if p.is_ascii_alphabetic() || p == '~' {
                                chars.next();
                                break;
                            }
                            chars.next();
                        }
                    }
                    Some(']') => {
                        chars.next();
                        while let Some(&p) = chars.peek() {
                            if p == '\x07' {
                                chars.next();
                                break;
                            }
                            if p == '\x1b' {
                                chars.next();
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                            chars.next();
                        }
                    }
                    _ => {
                        chars.next();
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}
