//! The Jev pre-filter: before continuous mode spends a Claude request on a
//! paragraph, Jev (TypeSafe's scoring model) grades how much it needs a
//! copy-edit, a fact-check and references, and only grades above the
//! threshold get acted on. Grading a paragraph costs a fraction of a cent.
//!
//! Jev also grades the whole document, now and then, on whether it looks
//! ready for a full look-over; above the threshold, the sidebar suggests a
//! review of the whole document. It never starts one by itself.
//!
//! The TypeSafe API key lives in the macOS Keychain, never in the settings
//! file. `WINGMAN_JEV_KEY` overrides it, for development.

use std::time::Duration;

use serde_json::{Value, json};

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

/// Grades paragraphs and keeps the TypeSafe key: [`Jev`] for real, or a fake
/// in tests, so they need neither the network nor the Keychain.
pub trait Grader: Send + Sync {
    fn load_key(&self) -> Result<Option<String>, String>;
    fn store_key(&self, key: &str) -> Result<(), String>;
    fn clear_key(&self) -> Result<(), String>;
    /// Blocks, so call it off the main thread.
    fn grade(&self, key: &str, model: &str, paragraph: &str) -> Result<Grades, JevError>;
    /// How ready the whole document looks for a full review, from 0 to 1.
    /// Blocks, so call it off the main thread.
    fn grade_readiness(&self, key: &str, model: &str, document: &str) -> Result<f32, JevError>;
}

/// TypeSafe's Jev, with the key in the macOS Keychain.
pub struct Jev;

impl Grader for Jev {
    fn load_key(&self) -> Result<Option<String>, String> {
        let result = load_key();
        match &result {
            Ok(Some(key)) => {
                // Keep it out of the log, whatever ends up quoting it.
                wingman_log::add_secret(key);
                log::info!(target: "jev", "TypeSafe key found ({} characters)", key.len());
            }
            Ok(None) => log::info!(target: "jev", "no TypeSafe key in the Keychain"),
            Err(e) => {
                log::error!(target: "jev", "couldn't read the TypeSafe key from the Keychain: {e}")
            }
        }
        result
    }
    fn store_key(&self, key: &str) -> Result<(), String> {
        wingman_log::add_secret(key);
        let result = store_key(key);
        match &result {
            Ok(()) => log::info!(target: "jev", "saved a TypeSafe key in the Keychain"),
            Err(e) => log::error!(target: "jev", "couldn't save the TypeSafe key: {e}"),
        }
        result
    }
    fn clear_key(&self) -> Result<(), String> {
        let result = clear_key();
        match &result {
            Ok(()) => log::info!(target: "jev", "removed the TypeSafe key from the Keychain"),
            Err(e) => log::error!(target: "jev", "couldn't remove the TypeSafe key: {e}"),
        }
        result
    }
    fn grade(&self, key: &str, model: &str, paragraph: &str) -> Result<Grades, JevError> {
        grade(key, model, paragraph)
    }
    fn grade_readiness(&self, key: &str, model: &str, document: &str) -> Result<f32, JevError> {
        grade_readiness(key, model, document)
    }
}

/// The questions, each a rubric from "not at all" to "a lot". Scores are
/// normalised to 0..1 by level, so a rubric's wording decides which levels
/// clear the threshold (0.66 by default). The first three are asked of a
/// paragraph, `full_review` of the whole document.
const QUESTIONS: [(&str, &str, &[&str]); 4] = [
    (
        "copy_edit",
        "You are a professional copy editor at a high-profile publication, judging a \
         paragraph from a blog post written in Markdown. It is often dictated, so it can \
         contain transcription errors. Hold it to your publication's standard: how much \
         copy-editing does it need before it could run? Any misspelling, typo, wrong or \
         missing word, grammar mistake or punctuation error is an error, however small: \
         even a single one means the paragraph needs an edit. Check the spelling of every word \
         before you judge the style. Ignore Markdown syntax.",
        // Three levels: 0, 50 and 100%. Any error at all is the top level,
        // so Jev's uncertainty between one slip and style alone still lands
        // well above the threshold; style alone is 50%.
        &[
            "No errors at all, and it reads cleanly",
            "No errors at all, but some phrasing is awkward, wordy or hard to follow",
            "At least one error: a misspelled word, typo, wrong or missing word, or grammar or punctuation slip",
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
    (
        "full_review",
        "You are the editor at a high-profile publication, looking at a writer's blog post \
         in Markdown to decide whether it is ready for a full editorial read: a copy-edit, \
         fact-check and source review of the whole piece at once, rather than paragraph by \
         paragraph as it is written. A full read pays off only on a complete draft whose \
         structure has settled: an opening, a developed body and an ending, all written, \
         with no placeholders, notes to self, TODOs, empty sections or sentences left \
         unfinished. An outline, a draft still being written, or one with whole sections \
         missing is not ready, however good its paragraphs are. Judge the piece as a whole, \
         not the quality of its prose.",
        // Five levels: 0, 25, 50, 75 and 100%. A complete draft is 75%.
        &[
            "Not started: notes, an outline or a few opening paragraphs",
            "Early: sections missing or unwritten, placeholders or notes to self",
            "Mostly written, but parts are unfinished or the structure is still moving",
            "A complete draft: opening, body and ending all written, with at most small gaps",
            "Finished: complete and settled, only a final read left before publishing",
        ],
    ),
];

/// The questions asked of each paragraph.
const PARAGRAPH_QUESTIONS: [&str; 3] = ["copy_edit", "fact_check", "references"];
/// The question asked of the whole document.
const DOCUMENT_QUESTION: &str = "full_review";

/// Jev reads at most this much of a document; a longer one is sent as a
/// sketch of its outline instead (see [`document_sketch`]).
const MAX_DOCUMENT_BYTES: usize = 30_000;

pub fn request_body(model: &str, paragraph: &str) -> Value {
    body_for(model, paragraph, &PARAGRAPH_QUESTIONS)
}

/// The request for the whole document's readiness for a full review.
pub fn readiness_request_body(model: &str, document: &str) -> Value {
    body_for(model, &document_sketch(document), &[DOCUMENT_QUESTION])
}

fn body_for(model: &str, state: &str, names: &[&str]) -> Value {
    let questions: serde_json::Map<String, Value> = QUESTIONS
        .iter()
        .filter(|(name, _, _)| names.contains(name))
        .map(|(name, instructions, criteria)| {
            (
                name.to_string(),
                json!({ "type": "score", "instructions": instructions, "criteria": criteria }),
            )
        })
        .collect();
    json!({ "model": model, "state": state, "questions": questions })
}

/// A document short enough for Jev as it is, or else its shape: every
/// heading, with the first and last lines of the prose under it, so Jev can
/// still judge whether the piece is complete and settled.
pub fn document_sketch(document: &str) -> String {
    if document.len() <= MAX_DOCUMENT_BYTES {
        return document.to_string();
    }
    let mut out = String::from(
        "[A long post, shown as its outline: each heading with the start and end of the text under it.]\n\n",
    );
    let mut section: Vec<&str> = Vec::new();
    let flush = |section: &mut Vec<&str>, out: &mut String| {
        let prose: Vec<&str> = section
            .iter()
            .copied()
            .filter(|l| !l.trim().is_empty())
            .collect();
        match prose.len() {
            0 => out.push_str("[nothing written yet]\n\n"),
            1 | 2 => out.push_str(&format!("{}\n\n", prose.join("\n"))),
            n => out.push_str(&format!(
                "{}\n[… {} more lines …]\n{}\n\n",
                prose[0],
                n - 2,
                prose[n - 1]
            )),
        }
        section.clear();
    };
    for line in document.lines() {
        if line.trim_start().starts_with('#') {
            flush(&mut section, &mut out);
            out.push_str(line.trim());
            out.push('\n');
        } else {
            section.push(line);
        }
    }
    flush(&mut section, &mut out);
    if out.len() > MAX_DOCUMENT_BYTES {
        let mut end = MAX_DOCUMENT_BYTES;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push_str("\n[… the outline goes on …]");
    }
    out
}

/// One answer's score, normalised from its rubric's levels to 0..1.
fn score_of(reply: &Value, name: &str) -> Result<f32, String> {
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
}

/// Reads the readiness score for a full review.
pub fn parse_readiness(reply: &Value) -> Result<f32, String> {
    score_of(reply, DOCUMENT_QUESTION)
}

/// Reads the three scores, each normalised from its rubric's levels to 0..1.
pub fn parse_grades(reply: &Value) -> Result<Grades, String> {
    let score = |name: &str| score_of(reply, name);
    Ok(Grades {
        copy_edit: score("copy_edit")?,
        fact_check: score("fact_check")?,
        references: score("references")?,
    })
}

/// Grades a paragraph. Blocks, so call it off the main thread.
pub fn grade(key: &str, model: &str, paragraph: &str) -> Result<Grades, JevError> {
    if std::env::var_os("WINGMAN_FAKE").is_some() {
        log::debug!(target: "jev", "WINGMAN_FAKE: canned grades");
        return Ok(fake_grades(paragraph));
    }
    let body = call(key, request_body(model, paragraph))?;
    parse_grades(&body).map_err(JevError::Other)
}

/// Grades how ready the whole document is for a full review. Blocks, so
/// call it off the main thread.
pub fn grade_readiness(key: &str, model: &str, document: &str) -> Result<f32, JevError> {
    if std::env::var_os("WINGMAN_FAKE").is_some() {
        log::debug!(target: "jev", "WINGMAN_FAKE: canned readiness");
        return Ok(fake_readiness(document));
    }
    let body = call(key, readiness_request_body(model, document))?;
    parse_readiness(&body).map_err(JevError::Other)
}

/// Sends a request to Jev and returns its reply.
fn call(key: &str, request: Value) -> Result<Value, JevError> {
    let started = std::time::Instant::now();
    let questions: Vec<String> = request["questions"]
        .as_object()
        .map(|q| q.keys().cloned().collect())
        .unwrap_or_default();
    let state_bytes = request["state"].as_str().map_or(0, str::len);
    log::debug!(
        target: "jev",
        "asking {} ({}) about {state_bytes} bytes",
        request["model"].as_str().unwrap_or("?"),
        questions.join(", ")
    );
    let result = send(key, request);
    match &result {
        Ok(_) => log::debug!(target: "jev", "answered in {:?}", started.elapsed()),
        Err(e) => log::warn!(target: "jev", "failed after {:?}: {e:?}", started.elapsed()),
    }
    result
}

fn send(key: &str, request: Value) -> Result<Value, JevError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent
        .post(ENDPOINT)
        .header("Authorization", &format!("Bearer {key}"))
        .send_json(request)
        .map_err(|e| JevError::Other(format!("Couldn't reach Jev: {e}")))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok());
    let body: Value = response.body_mut().read_json().unwrap_or(Value::Null);
    match status {
        200 => Ok(body),
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

/// Canned readiness for `WINGMAN_FAKE=1`: ready once there are a few
/// paragraphs and no "TODO".
fn fake_readiness(document: &str) -> f32 {
    let paragraphs = document
        .split("\n\n")
        .filter(|p| p.split_whitespace().count() >= 6)
        .count();
    if paragraphs >= 3 && !document.contains("TODO") {
        0.8
    } else {
        0.2
    }
}

fn keychain() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|e| e.to_string())
}

/// The stored key, if there is one.
pub fn load_key() -> Result<Option<String>, String> {
    if let Some(key) = std::env::var_os("WINGMAN_JEV_KEY") {
        log::info!(target: "jev", "using the TypeSafe key from WINGMAN_JEV_KEY");
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
    use super::{
        MAX_DOCUMENT_BYTES, document_sketch, grade, grade_readiness, load_key, parse_grades,
        parse_readiness, readiness_request_body, request_body,
    };
    use serde_json::json;

    #[test]
    fn asks_three_score_questions() {
        let body = request_body("jev-1.14.0", "Some text.");
        assert_eq!(body["model"], "jev-1.14.0");
        assert_eq!(body["state"], "Some text.");
        for (name, levels) in [("copy_edit", 3), ("fact_check", 6), ("references", 6)] {
            assert_eq!(body["questions"][name]["type"], "score");
            assert_eq!(
                body["questions"][name]["criteria"]
                    .as_array()
                    .unwrap()
                    .len(),
                levels
            );
        }
        assert!(body["questions"].get("full_review").is_none());
    }

    #[test]
    fn asks_the_whole_document_one_question() {
        let body = readiness_request_body("jev-1.14.0", "# A post\n\nSome text.");
        assert_eq!(body["state"], "# A post\n\nSome text.");
        let questions = body["questions"].as_object().unwrap();
        assert_eq!(questions.len(), 1);
        assert_eq!(questions["full_review"]["type"], "score");
        assert_eq!(
            questions["full_review"]["criteria"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let reply = json!({"answers": {"full_review": {"score": 3.0}}});
        assert_eq!(parse_readiness(&reply).unwrap(), 0.75);
        // A complete draft clears the default threshold; a mostly written one doesn't.
        let at = |level: f64| {
            parse_readiness(&json!({"answers": {"full_review": {"score": level}}})).unwrap()
        };
        assert!(at(2.0) < 0.66 && at(3.0) > 0.66);
    }

    #[test]
    fn sketches_a_long_document() {
        let body = "A line of prose that goes on for a while, to take up room.\n".repeat(600);
        let doc = format!(
            "# Title\n\nIntro line.\n\n## Middle\n\n{body}\n## Empty\n\n## End\n\nThe last line."
        );
        assert!(doc.len() > MAX_DOCUMENT_BYTES);
        let sketch = document_sketch(&doc);
        assert!(sketch.len() < 2_000, "{}", sketch.len());
        for part in [
            "# Title",
            "Intro line.",
            "## Middle",
            "more lines",
            "## Empty",
            "[nothing written yet]",
            "## End",
            "The last line.",
        ] {
            assert!(sketch.contains(part), "{part} missing from {sketch}");
        }
        // A short one goes as it is.
        assert_eq!(document_sketch("# Short\n\nText."), "# Short\n\nText.");
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
        assert!(at(1.0) < 0.66, "style alone stays below");
        assert!(at(2.0) > 0.66, "a single error clears it");
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
            (
                "typos",
                [true, false, false],
                "I went to the confrence last weak and it was realy usefull. I enjoyed the talks alot and met some grate people.",
            ),
            (
                "one typo",
                [true, false, false],
                "I spent Saturday morning rewriting the parser, and by lunch it handled every file I recieved from the team.",
            ),
            (
                "one slip",
                [true, false, false],
                "We moved the build to the new machine last month, and its been noticeably faster ever since.",
            ),
            (
                "clean",
                [false, false, false],
                "I spent Saturday morning rewriting the parser, and by lunch it handled every file I received from the team.",
            ),
            (
                "expert view",
                [false, true, true],
                "Most of the growth in cloud spending now comes from AI workloads, and the big providers are building data centres faster than the power grid can keep up.",
            ),
            (
                "statistic",
                [false, true, true],
                "Rust has topped Stack Overflow's survey as the most admired language for nine years running.",
            ),
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

        // Whole documents, with whether each should clear 0.66 for a full review.
        let finished = "# Why I moved my builds to a Mac mini\n\nFor years I built everything on a laptop that sounded like a jet engine. Last spring I finally moved the builds to a Mac mini under my desk, and I want to explain why it worked better than I expected.\n\n## The problem\n\nMy laptop throttled within minutes of a full build. A clean build of the editor took eleven minutes, and the fans made calls impossible while it ran.\n\n## What changed\n\nThe mini builds the same project in four minutes, silently, while I keep working on the laptop. I push a branch, the mini picks it up, and the results arrive in my terminal before I have finished my coffee.\n\n## Was it worth it?\n\nIt paid for itself within a couple of months in time I no longer spend waiting. If your builds take longer than a cup of coffee, a small dedicated machine is worth a look.\n";
        let early = "# Why I moved my builds to a Mac mini\n\nFor years I built everything on a laptop that sounded like a jet engine.\n\n## The problem\n\nTODO: numbers from the old laptop\n\n## What changed\n\n- faster\n- quieter\n- talk about the setup?\n\n## Conclusion\n";
        let half = "# Why I moved my builds to a Mac mini\n\nFor years I built everything on a laptop that sounded like a jet engine. Last spring I finally moved the builds to a Mac mini under my desk, and I want to explain why it worked better than I expected.\n\n## The problem\n\nMy laptop throttled within minutes of a full build. A clean build of the editor took eleven minutes, and the fans made calls impossible while it ran.\n\n## What changed\n\nThe mini builds the same project in four minutes, and\n\n## Was it worth it?\n\n(come back to this once I have the numbers)\n";
        let essay = "# Three things dictation taught me about writing\n\nI started dictating my posts last year because my wrists hurt. I expected to lose something in the switch. Instead I picked up three habits I now rely on, even when I type.\n\nThe first is that I write the way I talk, and that turns out to be a good thing. Sentences come out shorter. I stop hedging, because hedging out loud sounds ridiculous.\n\nThe second is that a first draft is just raw material. Dictation produces a mess: repeated words, odd punctuation, the occasional misheard phrase. I stopped expecting the first pass to be right, and editing became the real work.\n\nThe third is that pauses matter. When I stop talking, it is usually because I don't yet know what I think. I now treat those pauses as a signal to step away rather than push through.\n\nNone of this needs a microphone. But it took one for me to notice it, and my writing is better for it.\n";
        let notes = "# Post idea: build machines\n\n- laptop too loud\n- mini is faster (numbers?)\n- cost vs time saved\n- TODO: intro\n- maybe a section on CI\n";
        let mut wrong = Vec::new();
        for (name, expect, doc) in [
            ("finished post", true, finished),
            ("short essay", true, essay),
            ("half-written", false, half),
            ("early draft", false, early),
            ("notes", false, notes),
        ] {
            let r = grade_readiness(&key, "jev-1.14.0", doc).unwrap();
            println!("{name:>13}: full review {:.0}%", r * 100.);
            if (r > 0.66) != expect {
                wrong.push(format!("{name} at {:.0}%", r * 100.));
            }
        }
        assert!(wrong.is_empty(), "misgraded: {wrong:?}");
    }
}
