#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    NoKey,
    Refused,
    Forbidden,
    Unreachable,
    BadAnswer,
    RateLimited,
}

impl Fault {
    pub const ALL: [Fault; 6] = [
        Fault::NoKey,
        Fault::Refused,
        Fault::Forbidden,
        Fault::Unreachable,
        Fault::BadAnswer,
        Fault::RateLimited,
    ];

    pub fn parse(error: &str) -> Option<(Fault, &str)> {
        let (word, message) = error.split_once(": ")?;
        let fault = Fault::ALL.into_iter().find(|f| f.word() == word)?;
        Some((fault, message))
    }

    pub fn word(self) -> &'static str {
        match self {
            Fault::NoKey => "no_key",
            Fault::Refused => "refused",
            Fault::Forbidden => "forbidden",
            Fault::Unreachable => "unreachable",
            Fault::BadAnswer => "bad_answer",
            Fault::RateLimited => "rate_limited",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub fault: Fault,
    pub message: String,
    pub retry_after: Option<i64>,
}

impl Failure {
    pub fn new(fault: Fault, message: impl Into<String>) -> Self {
        Failure {
            fault,
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn until(mut self, at: Option<i64>) -> Self {
        self.retry_after = at;
        self
    }

    pub fn text(&self) -> String {
        format!("{}: {}", self.fault.word(), self.message)
    }
}
