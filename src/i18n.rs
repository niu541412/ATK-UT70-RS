#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Chinese,
    English,
}

impl Language {
    pub fn pick(self, chinese: &'static str, english: &'static str) -> &'static str {
        match self {
            Self::Chinese => chinese,
            Self::English => english,
        }
    }
}

#[derive(Clone)]
pub struct Message {
    chinese: String,
    english: String,
}

impl Message {
    pub fn new(chinese: impl Into<String>, english: impl Into<String>) -> Self {
        Self {
            chinese: chinese.into(),
            english: english.into(),
        }
    }

    pub fn text(&self, language: Language) -> &str {
        match language {
            Language::Chinese => &self.chinese,
            Language::English => &self.english,
        }
    }
}
