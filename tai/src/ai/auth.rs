pub struct ChainedAuth {
    api_key: Option<String>,
}

impl ChainedAuth {
    pub fn new(config_key: Option<String>) -> Self {
        let api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .or(config_key.filter(|k| !k.is_empty()));
        ChainedAuth { api_key }
    }

    pub fn get_api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }
}
