//! Wire types shared with the service. Keep in sync with `service/src/protocol.ts`.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    CopyEdit,
    FactCheck,
    FindReferences,
}

/// Which credential the service's Claude Code uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// The user's Claude plan, through their `claude` login. API keys in the
    /// environment are ignored, so usage never bills the API by accident.
    #[default]
    Subscription,
    /// `ANTHROPIC_API_KEY`, billed to the Claude API. Subscription tokens in
    /// the environment are ignored.
    ApiKey,
    /// Another Anthropic-compatible endpoint, such as OpenRouter or a local
    /// model server, set with the `WINGMAN_PROVIDER_*` variables. No Claude
    /// credential is passed on to it.
    Provider,
}

impl AuthMode {
    /// The value the service reads from `WINGMAN_AUTH`.
    pub fn as_env_value(self) -> &'static str {
        match self {
            AuthMode::Subscription => "subscription",
            AuthMode::ApiKey => "api_key",
            AuthMode::Provider => "provider",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Request<'a> {
    Hello {
        id: &'a str,
    },
    Task {
        id: &'a str,
        task: TaskKind,
        text: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<&'a str>,
    },
    Cancel {
        id: &'a str,
        target: &'a str,
    },
    Shutdown {
        id: &'a str,
    },
}

/// An event the service sends back for one request.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Hello {
        protocol: u32,
        version: String,
        auth: AuthMode,
        /// In provider mode: the provider's name and the model in use.
        #[serde(default)]
        provider: Option<String>,
        #[serde(default)]
        model: Option<String>,
    },
    /// A piece of the reply, to append to what has arrived so far.
    Delta {
        text: String,
    },
    /// Progress note, such as "Using WebSearch".
    Status {
        message: String,
    },
    /// The request finished. `text` is the reply as Markdown, or a one-line
    /// summary when `data` is set.
    Done {
        text: String,
        /// The structured reply, for tasks that have one (copy-edit and
        /// fact-check). See `service/src/tasks.ts` for the shapes.
        #[serde(default)]
        data: Option<serde_json::Value>,
    },
    Error {
        message: String,
    },
}

impl Event {
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            Event::Hello { .. } | Event::Done { .. } | Event::Error { .. }
        )
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Envelope {
    pub id: String,
    #[serde(flatten)]
    pub event: Event,
}
