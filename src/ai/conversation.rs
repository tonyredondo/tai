use async_openai::types::ChatCompletionRequestMessage;
use std::collections::VecDeque;

pub struct ConversationHistory {
    messages: VecDeque<ChatCompletionRequestMessage>,
    max_exchanges: usize,
}

impl ConversationHistory {
    pub fn new(max_exchanges: usize) -> Self {
        ConversationHistory {
            messages: VecDeque::new(),
            max_exchanges,
        }
    }

    pub fn push_user(&mut self, content: &str) {
        self.messages.push_back(
            ChatCompletionRequestMessage::User(
                async_openai::types::ChatCompletionRequestUserMessage {
                    content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                        content.to_string(),
                    ),
                    name: None,
                },
            ),
        );
        self.trim();
    }

    pub fn push_message(&mut self, msg: ChatCompletionRequestMessage) {
        self.messages.push_back(msg);
        self.trim();
    }

    pub fn push_assistant(&mut self, content: &str) {
        self.messages.push_back(
            ChatCompletionRequestMessage::Assistant(
                async_openai::types::ChatCompletionRequestAssistantMessage {
                    content: Some(
                        async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(
                            content.to_string(),
                        ),
                    ),
                    name: None,
                    tool_calls: None,
                    refusal: None,
                    audio: None,
                    #[allow(deprecated)]
                    function_call: None,
                },
            ),
        );
        self.trim();
    }

    pub fn push_assistant_tool_call(&mut self, tool_calls: Vec<async_openai::types::ChatCompletionMessageToolCall>) {
        self.messages.push_back(
            ChatCompletionRequestMessage::Assistant(
                async_openai::types::ChatCompletionRequestAssistantMessage {
                    content: None,
                    name: None,
                    tool_calls: Some(tool_calls),
                    refusal: None,
                    audio: None,
                    #[allow(deprecated)]
                    function_call: None,
                },
            ),
        );
    }

    pub fn push_tool_result(&mut self, tool_call_id: &str, content: &str) {
        self.messages.push_back(
            ChatCompletionRequestMessage::Tool(
                async_openai::types::ChatCompletionRequestToolMessage {
                    tool_call_id: tool_call_id.to_string(),
                    content: async_openai::types::ChatCompletionRequestToolMessageContent::Text(
                        content.to_string(),
                    ),
                },
            ),
        );
        self.trim();
    }

    pub fn build_messages(
        &mut self,
        system: ChatCompletionRequestMessage,
    ) -> Vec<ChatCompletionRequestMessage> {
        const MAX_CHARS: usize = 120_000;

        self.trim_by_size(MAX_CHARS);

        let mut msgs = Vec::with_capacity(1 + self.messages.len());
        msgs.push(system);
        msgs.extend(self.messages.iter().cloned());
        msgs
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn remove_trailing_orphans(&mut self) {
        loop {
            match self.messages.back() {
                Some(ChatCompletionRequestMessage::Assistant(a)) if a.tool_calls.is_some() && a.content.is_none() => {
                    self.messages.pop_back();
                }
                Some(ChatCompletionRequestMessage::Tool(_)) => {
                    self.messages.pop_back();
                }
                _ => break,
            }
        }
    }

    fn estimate_msg_chars(msg: &ChatCompletionRequestMessage) -> usize {
        match msg {
            ChatCompletionRequestMessage::System(m) => {
                match &m.content {
                    async_openai::types::ChatCompletionRequestSystemMessageContent::Text(t) => t.len(),
                    _ => 100,
                }
            }
            ChatCompletionRequestMessage::User(m) => {
                match &m.content {
                    async_openai::types::ChatCompletionRequestUserMessageContent::Text(t) => t.len(),
                    _ => 100,
                }
            }
            ChatCompletionRequestMessage::Assistant(m) => {
                let content_len = m.content.as_ref().map(|c| match c {
                    async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(t) => t.len(),
                    _ => 100,
                }).unwrap_or(0);
                let tools_len = m.tool_calls.as_ref().map(|tcs| {
                    tcs.iter().map(|tc| tc.function.arguments.len() + tc.function.name.len() + 50).sum::<usize>()
                }).unwrap_or(0);
                content_len + tools_len
            }
            ChatCompletionRequestMessage::Tool(m) => {
                match &m.content {
                    async_openai::types::ChatCompletionRequestToolMessageContent::Text(t) => t.len(),
                    _ => 100,
                }
            }
            _ => 100,
        }
    }

    fn trim(&mut self) {
        while self.messages.len() > self.max_exchanges * 2 {
            self.pop_front_safe();
        }
    }

    fn trim_by_size(&mut self, max_chars: usize) {
        while self.messages.len() > 2 {
            let total: usize = self.messages.iter().map(Self::estimate_msg_chars).sum();
            if total <= max_chars {
                break;
            }
            self.pop_front_safe();
        }
    }

    fn pop_front_safe(&mut self) {
        let front = self.messages.front();
        if let Some(ChatCompletionRequestMessage::Assistant(a)) = front {
            if a.tool_calls.is_some() && self.messages.len() > 1 {
                self.messages.pop_front();
                if let Some(ChatCompletionRequestMessage::Tool(_)) = self.messages.front() {
                    self.messages.pop_front();
                }
                return;
            }
        }
        self.messages.pop_front();
    }
}
