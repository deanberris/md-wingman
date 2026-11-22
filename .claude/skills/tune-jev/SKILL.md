---
name: tune-jev
description: Calibrate or change md-wingman's Jev pre-filter rubrics (TypeSafe scoring) and check them against live grades with the ignored jev_live test.
---

# Tune the Jev pre-filter

Jev is TypeSafe's scoring model. When "check as you write" is on with the pre-filter, md-wingman asks Jev to grade each
changed paragraph for copy-edit, fact-check and references, and only sends Claude a check when the grade clears the
threshold (`jev_threshold`, default 0.66). A fourth question, `full_review`, grades whether the whole document is ready
for a full look-over.

## Where things are

- `crates/md-wingman/src/jev.rs`: `QUESTIONS` holds each question's instructions and its criteria levels; the request
  goes to `https://api.typesafe.ai/v1/systemone` with model `jev-1.14.0`.
- A score is normalised as `score / (levels - 1)`. Jev returns an expected value, so with many levels a borderline
  paragraph smears across them. Fewer levels give sharper grades.
- `crates/md-wingman/src/continuous.rs` and `prefilter.rs`: what gets graded and when.
- API docs: <https://docs.typesafe.ai>. 429 and 529 mean back off.

## What Jev should do

- Judge like a professional copy editor, fact-checker and research editor at a high-profile publication.
- Any typo at all must clear 0.66 for copy-edit. Clean prose must not.
- Observations of the kind analysts or experts make score high for both fact-check and references.
- Readiness: a finished post scores high, a half-written draft or notes low.

## Calibrate

The `jev_live_calibration` test sends fixed samples (typos, one typo, clean, awkward, claims, expert observations, and
five readiness samples) and asserts which ones clear the threshold. It needs a TypeSafe key, so it is `#[ignore]`d and
runs on the Mac, where the key is in the Keychain:

```sh
cargo test -p md-wingman jev_live -- --ignored --nocapture
```

Run it through Remote Control on the maintainer's Mac (see `build-and-run`). It reads the key from the Keychain or
`WINGMAN_JEV_KEY`; never print, log or copy the key, and never ask the maintainer for it.

## Changing a rubric

1. Write down which sample is misgraded and what it should score.
2. Change the instructions or levels in `QUESTIONS`. Keep the professional-editor framing. If you change the number of
   levels, update the comment that lists their percentages.
3. Add a sample to `jev_live_calibration` for the case you are fixing.
4. Run the live test at least twice (Jev isn't deterministic) and record the scores per sample in the PR.
5. The unit tests in `jev.rs` and `ui_tests/continuous.rs` use `FakeJev`, so they don't change with the rubric; update
   them only if the request shape changes.
