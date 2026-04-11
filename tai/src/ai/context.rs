use async_openai::types::ChatCompletionRequestMessage;
use std::path::Path;

pub struct ContextBuilder {
    pub max_context_lines: usize,
}

impl ContextBuilder {
    pub fn new(max_context_lines: usize) -> Self {
        ContextBuilder { max_context_lines }
    }

    pub fn build_system_message(
        os: &str,
        arch: &str,
        shell: &str,
        cwd: &Path,
    ) -> ChatCompletionRequestMessage {
        let content = format!(
            "You are TAI, an AI assistant embedded in a terminal emulator. You can see \
             the user's terminal output and execute commands directly.\n\n\
             Environment:\n\
             - OS: {os} ({arch})\n\
             - Shell: {shell}\n\
             - Working directory: {}\n\n\
             Rules:\n\
             - When the user asks you to do something, prefer using the run_command tool \
             to execute commands directly rather than just explaining what to do.\n\
             - You can chain multiple run_command calls to complete complex tasks.\n\
             - Be concise. You are in a terminal, not a chat window.\n\
             - After running commands, keep your response very brief since the user \
             already sees the command output in real time. Only add commentary if \
             something needs explanation.\n\
             - When using run_command, always include a brief explanation parameter.",
            cwd.display()
        );

        ChatCompletionRequestMessage::System(
            async_openai::types::ChatCompletionRequestSystemMessage {
                content: async_openai::types::ChatCompletionRequestSystemMessageContent::Text(
                    content,
                ),
                name: None,
            },
        )
    }

    pub fn build_user_message(
        &self,
        query: &str,
        terminal_buffer: &str,
        command_history: &[String],
    ) -> ChatCompletionRequestMessage {
        let lines: Vec<&str> = terminal_buffer.lines().collect();
        let start = if lines.len() > self.max_context_lines {
            lines.len() - self.max_context_lines
        } else {
            0
        };
        let trimmed = lines[start..].join("\n");

        let history_str = if command_history.is_empty() {
            "none".to_string()
        } else {
            command_history.join(", ")
        };

        let content = format!(
            "Terminal output (last {} lines):\n```\n{}\n```\n\nRecent commands: {}\n\nUser: {}",
            self.max_context_lines, trimmed, history_str, query
        );

        ChatCompletionRequestMessage::User(
            async_openai::types::ChatCompletionRequestUserMessage {
                content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(content),
                name: None,
            },
        )
    }
}
