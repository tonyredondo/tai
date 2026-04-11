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
        &self,
        system: ChatCompletionRequestMessage,
    ) -> Vec<ChatCompletionRequestMessage> {
        let mut msgs = Vec::with_capacity(1 + self.messages.len());
        msgs.push(system);
        msgs.extend(self.messages.iter().cloned());
        msgs
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    fn trim(&mut self) {
        while self.messages.len() > self.max_exchanges * 2 {
            let front = self.messages.front();
            if let Some(ChatCompletionRequestMessage::Assistant(a)) = front {
                if a.tool_calls.is_some() && self.messages.len() > 1 {
                    self.messages.pop_front();
                    if let Some(ChatCompletionRequestMessage::Tool(_)) = self.messages.front() {
                        self.messages.pop_front();
                    }
                    continue;
                }
            }
            self.messages.pop_front();
        }
    }
}
