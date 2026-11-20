//! The Jev pre-filter: before continuous mode spends a Claude request on a
//! paragraph, Jev (TypeSafe's scoring model) grades how much it needs a
//! copy-edit, a fact-check and references, and only grades above the
//! threshold get acted on. Grading a paragraph costs a fraction of a cent.
//!
//! The TypeSafe API key lives in the macOS Keychain, never in the settings
//! file. `WINGMAN_JEV_KEY` overrides it, for development.

use std::time::Duration;

use serde_json::{json, Value};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const KEYCHAIN_SERVICE: &str = "md-wingman";
const KEYCHAIN_ACCOUNT: &str = "typesafe-api-key";

/// How much a paragraph needs each kind of help, from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grades {
    pub copy_edit: f32,
    pub fact_check: f32,
    pub references: f32,
}

impl Grades {
    pub fn summary(&self) -> String {
        format!(
            "copy-edit {:.0}% · fact-check {:.0}% · references {:.0}%",
            self.copy_edit * 100.,
            self.fact_check * 100.,
            self.references * 100.
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JevError {
    /// The key was rejected.
    BadKey,
    /// Rate-limited or overloaded: try again later.
    Busy {
        retry_after: Option<u64>,
    },
    Other(String),
}

impl std::fmt::Display for JevError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JevError::BadKey => write!(f, "TypeSafe didn't accept the API key"),
            JevError::Busy { .. } => write!(f, "Jev is busy"),
            JevError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// The three questions, each a rubric from "not at all" to "a lot".
const QUESTIONS: [(&str, &str, [&str; 4]); 3] = [
    (
        "copy_edit",
        "This is a paragraph from a blog post written in Markdown, often dictated, so it can \
         contain transcription errors. How much copy-editing does it need: spelling, grammar, \
         punctuation, wrong words and awkward phrasing? Ignore Markdown syntax.",
        [
            "None: it reads cleanly",
            "Minor: a slip or two that few readers would notice",
            "Some: clear errors or awkward sentences a reader would notice",
            "A lot: many errors, or hard to read",
        ],
    ),
    (
        "fact_check",
        "This is a paragraph from a blog post. Does it make factual claims worth checking \
         against sources: dates, numbers, names, events, quotes or statements about the world \
         that could be wrong?",
        [
            "None: opinion, narrative or instructions only",
            "Only well-known facts that are very unlikely to be wrong",
            "Some specific claims a careful reader might question",
            "Many specific claims, or claims that look wrong",
        ],
    ),
    (
        "references",
        "This is a paragraph from a blog post. Would it be stronger with a citation or a link \
         to a source, for a statistic, quote, study, news item or specific claim?",
        [
            "No: nothing a reader would want a source for",
            "Unlikely: a source would add little",
            "Probably: a claim or quote a reader may want a source for",
            "Yes: statistics, quotes, research or news that should be cited",
        ],
    ),
];

pub fn request_body(model: &str, paragraph: &str) -> Value {
    let questions: serde_json::Map<String, Value> = QUESTIONS
        .iter()
        .map(|(name, instructions, criteria)| {
            (
                name.to_string(),
                json!({ "type": "score", "instructions": instructions, "criteria": criteria }),
            )
        })
        .collect();
    json!({ "model": model, "state": paragraph, "questions": questions })
}

/// Reads the three scores, each normalised from its rubric's levels to 0..1.
pub fn parse_grades(reply: &Value) -> Result<Grades, String> {
    let score = |name: &str| -> Result<f32, String> {
        let answer = &reply["answers"][name];
        let score = answer["score"]
            .as_f64()
            .ok_or_else(|| format!("Jev's reply has no score for {name}"))?;
        // The top level is the number of rubric levels less one.
        let levels = answer["legend"]
            .as_object()
            .map(|l| l.len())
            .filter(|&n| n >= 2)
            .unwrap_or(QUESTIONS[0].2.len());
        Ok((score / (levels - 1) as f64).clamp(0., 1.) as f32)
    };
    Ok(Grades {
        copy_edit: score("copy_edit")?,
        fact_check: score("fact_check")?,
        references: score("references")?,
    })
}

/// Grades a paragraph. Blocks, so call it off the main thread.
pub fn grade(key: &str, model: &str, paragraph: &str) -> Result<Grades, JevError> {
    if std::env::var_os("WINGMAN_FAKE").is_some() {
        return Ok(fake_grades(paragraph));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent
        .post(ENDPOINT)
        .header("Authorization", &format!("Bearer {key}"))
        .send_json(request_body(model, paragraph))
        .map_err(|e| JevError::Other(format!("Couldn't reach Jev: {e}")))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok());
    let body: Value = response.body_mut().read_json().unwrap_or(Value::Null);
    match status {
        200 => parse_grades(&body).map_err(JevError::Other),
        401 | 403 => Err(JevError::BadKey),
        429 | 529 | 503 => Err(JevError::Busy { retry_after }),
        _ => Err(JevError::Other(format!(
            "Jev answered {status}: {}",
            body.get("detail")
                .map(|d| d.to_string())
                .unwrap_or_else(|| "no details".into())
        ))),
    }
}

/// Canned grades for `WINGMAN_FAKE=1`, from simple clues in the text.
fn fake_grades(paragraph: &str) -> Grades {
    let lower = paragraph.to_lowercase();
    let has = |clues: &[&str]| clues.iter().any(|c| lower.contains(c));
    Grades {
        copy_edit: if has(&["their are", "it were", " teh ", "  "]) {
            0.9
        } else {
            0.2
        },
        fact_check: if paragraph.chars().any(|c| c.is_ascii_digit()) {
            0.8
        } else {
            0.1
        },
        references: if has(&["%", "according to", "study", "said"]) {
            0.85
        } else {
            0.1
        },
    }
}

fn keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|e| e.to_string())
}

/// The stored key, if there is one.
pub fn load_key() -> Result<Option<String>, String> {
    if let Some(key) = std::env::var_os("WINGMAN_JEV_KEY") {
        return Ok(Some(key.to_string_lossy().into_owned()));
    }
    match keychain()?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn store_key(key: &str) -> Result<(), String> {
    keychain()?.set_password(key).map_err(|e| e.to_string())
}

pub fn clear_key() -> Result<(), String> {
    match keychain()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_grades, request_body};
    use serde_json::json;

    #[test]
    fn asks_three_score_questions() {
        let body = request_body("jev-1.14.0", "Some text.");
        assert_eq!(body["model"], "jev-1.14.0");
        assert_eq!(body["state"], "Some text.");
        for name in ["copy_edit", "fact_check", "references"] {
            assert_eq!(body["questions"][name]["type"], "score");
            assert_eq!(
                body["questions"][name]["criteria"]
                    .as_array()
                    .unwrap()
                    .len(),
                4
            );
        }
    }

    #[test]
    fn normalises_scores_by_their_levels() {
        let legend = json!({"0": "a", "1": "b", "2": "c", "3": "d"});
        let reply = json!({"answers": {
            "copy_edit": {"type": "score", "score": 3.0, "legend": legend},
            "fact_check": {"type": "score", "score": 1.5, "legend": legend},
            "references": {"type": "score", "score": 0.0, "legend": {"0": "a", "1": "b"}},
        }});
        let g = parse_grades(&reply).unwrap();
        assert_eq!(g.copy_edit, 1.0);
        assert_eq!(g.fact_check, 0.5);
        assert_eq!(g.references, 0.0);
        assert!(parse_grades(&json!({"answers": {}})).is_err());
    }
}
