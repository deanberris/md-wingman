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

/// The three questions, each a rubric from "not at all" to "a lot". Scores
/// are normalised to 0..1 by level, so a rubric's wording decides which
/// levels clear the threshold (0.66 by default).
const QUESTIONS: [(&str, &str, &[&str]); 3] = [
    (
        "copy_edit",
        "You are a professional copy editor at a high-profile publication, judging a \
         paragraph from a blog post written in Markdown. It is often dictated, so it can \
         contain transcription errors. Hold it to your publication's standard: how much \
         copy-editing does it need before it could run? Any misspelling, typo, wrong or \
         missing word, grammar mistake or punctuation error is an error, however small: \
         even a single one means the paragraph needs an edit. Check the spelling of every word \
         before you judge the style. Ignore Markdown syntax.",
        // Six levels: 0, 20, 40, 60, 80 and 100%. Any real error is 80% or
        // more; style alone stays at 60% or less.
        &[
            "No errors at all, and it reads cleanly",
            "No errors at all; a word choice could be polished",
            "No errors at all; some phrasing is awkward or wordy",
            "No errors at all, but a sentence is hard to follow",
            "One error: a single misspelled word, typo, wrong or missing word, or grammar or punctuation slip",
            "Two or more errors, or hard to read",
        ],
    ),
    (
        "fact_check",
        "You are a fact-checker at a high-profile publication, judging a paragraph from a \
         blog post. Does it make claims worth checking against sources? Anything said about \
         the world beyond the writer's own experience is a claim: dates, numbers, names, \
         events and quotes, and also observations, trends, comparisons and assessments of \
         the kind analysts, researchers or experts make. If an expert has already made the \
         point, it is worth checking.",
        // Six levels: 0, 20, 40, 60, 80 and 100%. Specific claims and
        // expert-style observations are 80% or more.
        &[
            "None: only the writer's own experience, opinion or instructions",
            "Only common knowledge nobody would question",
            "Vague statements with nothing specific to check",
            "A loose claim that is very probably right",
            "Claims worth checking: specific facts, or observations, trends or assessments of the kind analysts or experts make",
            "Many such claims, or claims that look wrong",
        ],
    ),
    (
        "references",
        "You are a research editor at a high-profile publication, judging a paragraph from a \
         blog post. Would it be stronger with a citation or a link to a source? Statistics, \
         quotes, studies and news need one, and so do observations, trends and assessments \
         that analysts, researchers or experts have already made: credit them and link to \
         their work.",
        // Six levels: 0, 20, 40, 60, 80 and 100%. Anything an expert has
        // already said is 80% or more.
        &[
            "No: only the writer's own experience or opinion",
            "Unlikely: common knowledge",
            "Possibly: a general point a link could support but doesn't need",
            "A point a link would help with, though readers wouldn't expect one",
            "Yes: observations, trends or assessments analysts or experts have made, or a claim readers would want a source for",
            "Definitely: statistics, quotes, research or news that must be cited",
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
        let asked = QUESTIONS
            .iter()
            .find(|(n, _, _)| *n == name)
            .map_or(4, |(_, _, c)| c.len());
        let answer = &reply["answers"][name];
        let score = answer["score"]
            .as_f64()
            .ok_or_else(|| format!("Jev's reply has no score for {name}"))?;
        // The top level is the number of rubric levels less one.
        let levels = answer["legend"]
            .as_object()
            .map(|l| l.len())
            .filter(|&n| n >= 2)
            .unwrap_or(asked);
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
    use super::{grade, load_key, parse_grades, request_body};
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
                6
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

    #[test]
    fn any_copy_edit_error_clears_the_threshold() {
        // Without a legend, the levels come from the rubric we sent.
        let at = |level: f64| {
            let reply = json!({"answers": {
                "copy_edit": {"score": level},
                "fact_check": {"score": 0.0},
                "references": {"score": 0.0},
            }});
            parse_grades(&reply).unwrap().copy_edit
        };
        assert!(at(3.0) < 0.66, "style alone stays below");
        assert!(at(4.0) > 0.66, "a single error clears it");
    }

    /// Calibration against real Jev, with the key from the Keychain or
    /// `WINGMAN_JEV_KEY`: `cargo test -p md-wingman jev_live -- --ignored --nocapture`
    #[test]
    #[ignore = "calls the TypeSafe API"]
    fn jev_live_calibration() {
        let key = load_key().unwrap().expect("no TypeSafe key");
        // Each sample with whether it should clear 0.66 for copy-edit,
        // fact-check and references.
        let samples = [
            ("typos", [true, false, false], "I went to the confrence last weak and it was realy usefull. I enjoyed the talks alot and met some grate people."),
            ("one typo", [true, false, false], "I spent Saturday morning rewriting the parser, and by lunch it handled every file I recieved from the team."),
            ("one slip", [true, false, false], "We moved the build to the new machine last month, and its been noticeably faster ever since."),
            ("clean", [false, false, false], "I spent Saturday morning rewriting the parser, and by lunch it handled every file I received from the team."),
            ("expert view", [false, true, true], "Most of the growth in cloud spending now comes from AI workloads, and the big providers are building data centres faster than the power grid can keep up."),
            ("statistic", [false, true, true], "Rust has topped Stack Overflow's survey as the most admired language for nine years running."),
        ];
        let mut wrong = Vec::new();
        for (name, expect, text) in samples {
            let g = grade(&key, "jev-1.14.0", text).unwrap();
            println!(
                "{name:>12}: copy-edit {:.0}%  fact-check {:.0}%  references {:.0}%",
                g.copy_edit * 100.,
                g.fact_check * 100.,
                g.references * 100.
            );
            let got = [g.copy_edit, g.fact_check, g.references].map(|v| v > 0.66);
            for (i, kind) in ["copy-edit", "fact-check", "references"].iter().enumerate() {
                if got[i] != expect[i] {
                    wrong.push(format!("{name}: {kind}"));
                }
            }
        }
        assert!(wrong.is_empty(), "misgraded: {wrong:?}");
    }
}
