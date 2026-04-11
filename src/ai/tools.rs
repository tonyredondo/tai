use async_openai::types::{ChatCompletionTool, ChatCompletionToolType, FunctionObject};
use serde_json::json;

pub fn run_command_tool() -> ChatCompletionTool {
    ChatCompletionTool {
        r#type: ChatCompletionToolType::Function,
        function: FunctionObject {
            name: "run_command".to_string(),
            description: Some("Execute a shell command in the user's terminal".to_string()),
            parameters: Some(json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute"
                    },
                    "explanation": {
                        "type": "string",
                        "description": "Why this command is being run"
                    }
                },
                "required": ["command"]
            })),
            strict: None,
        },
    }
}
