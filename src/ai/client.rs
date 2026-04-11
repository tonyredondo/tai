use async_openai::config::OpenAIConfig;
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionTool,
    CreateChatCompletionRequestArgs,
};
use async_openai::Client;
use futures::StreamExt;
use std::sync::mpsc;

use crate::ai::bridge::AiResponse;
use crate::ai::tools::run_command_tool;

pub struct AiClient {
    client: Client<OpenAIConfig>,
    model: String,
    tools: Vec<ChatCompletionTool>,
}

impl AiClient {
    pub fn new(api_key: &str, model: &str) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let client = Client::with_config(config);
        AiClient {
            client,
            model: model.to_string(),
            tools: vec![run_command_tool()],
        }
    }

    pub async fn chat_stream(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        response_tx: &mpsc::Sender<AiResponse>,
    ) -> Result<(), String> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(messages)
            .tools(self.tools.clone())
            .stream(true)
            .build()
            .map_err(|e| format!("Failed to build request: {e}"))?;

        let mut stream = self
            .client
            .chat()
            .create_stream(request)
            .await
            .map_err(|e| format!("API error: {e}"))?;

        let mut tool_call_id = String::new();
        let mut tool_call_name = String::new();
        let mut tool_call_args = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(response) => {
                    for choice in &response.choices {
                        if let Some(ref content) = choice.delta.content {
                            let _ = response_tx.send(AiResponse::Token(content.to_string()));
                        }

                        if let Some(ref tool_calls) = choice.delta.tool_calls {
                            for tc in tool_calls {
                                if let Some(ref id) = tc.id {
                                    if !tool_call_name.is_empty() {
                                        let _ = response_tx.send(AiResponse::ToolCall {
                                            id: tool_call_id.clone(),
                                            name: tool_call_name.clone(),
                                            arguments: tool_call_args.clone(),
                                        });
                                    }
                                    tool_call_id = id.to_string();
                                    tool_call_name.clear();
                                    tool_call_args.clear();
                                }
                                if let Some(ref func) = tc.function {
                                    if let Some(ref name) = func.name {
                                        tool_call_name = name.to_string();
                                    }
                                    if let Some(ref args) = func.arguments {
                                        tool_call_args.push_str(args);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = response_tx.send(AiResponse::Error(format!("Stream error: {e}")));
                    return Ok(());
                }
            }
        }

        if !tool_call_name.is_empty() {
            let _ = response_tx.send(AiResponse::ToolCall {
                id: tool_call_id,
                name: tool_call_name,
                arguments: tool_call_args,
            });
        }

        let _ = response_tx.send(AiResponse::Done);
        Ok(())
    }
}
