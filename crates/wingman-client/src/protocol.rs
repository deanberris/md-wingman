//! Wire types shared with the service. Keep in sync with `service/src/protocol.ts`.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    CopyEdit,
    FactCheck,
    FindReferences,
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Hello {
        protocol: u32,
        version: String,
    },
    /// A piece of the reply, to append to what has arrived so far.
    Delta {
        text: String,
    },
    /// Progress note, such as "Using WebSearch".
    Status {
        message: String,
    },
    /// The request finished. `text` is the full reply.
    Done {
        text: String,
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
