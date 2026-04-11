use async_openai::types::ChatCompletionRequestMessage;
use std::sync::mpsc;
use std::thread::JoinHandle;

use crate::ai::client::AiClient;
use crate::config::AiConfig;

pub enum AiRequest {
    Chat {
        messages: Vec<ChatCompletionRequestMessage>,
    },
    Cancel,
}

#[derive(Clone, Debug)]
pub enum AiResponse {
    Token(String),
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    Done,
    Error(String),
}

pub struct AiBridge {
    request_tx: mpsc::Sender<AiRequest>,
    response_rx: mpsc::Receiver<AiResponse>,
    _runtime_thread: JoinHandle<()>,
}

impl AiBridge {
    pub fn new(config: &AiConfig, api_key: &str) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<AiRequest>();
        let (response_tx, response_rx) = mpsc::channel::<AiResponse>();

        let model = config.model.clone();
        let key = api_key.to_string();

        let runtime_thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            let client = AiClient::new(&key, &model);

            rt.block_on(async {
                while let Ok(request) = request_rx.recv() {
                    match request {
                        AiRequest::Chat { messages } => {
                            if let Err(e) = client.chat_stream(messages, &response_tx).await {
                                let _ = response_tx.send(AiResponse::Error(e));
                            }
                        }
                        AiRequest::Cancel => {
                            // Cancellation drops the current stream in the next iteration
                        }
                    }
                }
            });
        });

        AiBridge {
            request_tx,
            response_rx,
            _runtime_thread: runtime_thread,
        }
    }

    pub fn send(&self, request: AiRequest) {
        let _ = self.request_tx.send(request);
    }

    pub fn try_recv(&self) -> Option<AiResponse> {
        self.response_rx.try_recv().ok()
    }
}
