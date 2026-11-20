//! Hard-wrapping Markdown paragraphs to a column, keeping list markers,
//! block quotes and indentation.

/// Rewraps `text` (one or more paragraphs) so no line is longer than `width`
/// characters where it can be helped. Fenced code, headings, tables, HTML and
/// blank lines are left as they are. Returns `None` if nothing would change.
pub fn reflow(text: &str, width: usize) -> Option<String> {
    let width = width.max(20);
    let mut out: Vec<String> = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    let mut in_fence = false;

    let flush = |block: &mut Vec<&str>, out: &mut Vec<String>| {
        if !block.is_empty() {
            out.extend(wrap_block(block, width));
            block.clear();
        }
    };

    for line in text.split('\n') {
        let bare = strip_quotes(line).1.trim_start();
        if bare.starts_with("```") || bare.starts_with("~~~") {
            flush(&mut block, &mut out);
            in_fence = !in_fence;
            out.push(line.to_string());
            continue;
        }
        if in_fence || keep_as_is(bare) || line.starts_with("    ") || line.starts_with('\t') {
            flush(&mut block, &mut out);
            out.push(line.to_string());
            continue;
        }
        // A list item starts a new block, and so does a change of quote depth.
        let starts_item = list_marker(bare).is_some();
        let depth_changed = block
            .first()
            .is_some_and(|first| strip_quotes(first).0 != strip_quotes(line).0);
        if starts_item || depth_changed {
            flush(&mut block, &mut out);
        }
        block.push(line);
        // A hard line break ends the block after this line.
        if line.ends_with("  ") || line.ends_with('\\') {
            flush(&mut block, &mut out);
        }
    }
    flush(&mut block, &mut out);

    let result = out.join("\n");
    (result != text).then_some(result)
}

/// Lines that are never rewrapped.
fn keep_as_is(bare: &str) -> bool {
    bare.is_empty()
        || bare.starts_with('#')
        || bare.starts_with('|')
        || bare.starts_with('<')
        || bare.starts_with("[^") && bare.contains("]:")
        || is_rule(bare)
}

fn is_rule(bare: &str) -> bool {
    let chars: Vec<char> = bare.chars().filter(|c| !c.is_whitespace()).collect();
    chars.len() >= 3
        && (chars.iter().all(|&c| c == '-')
            || chars.iter().all(|&c| c == '*')
            || chars.iter().all(|&c| c == '_'))
}

/// Splits leading indentation and `>` markers from a line: (prefix, rest).
fn strip_quotes(line: &str) -> (&str, &str) {
    let mut end = 0;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b'>' => {
                i += 1;
                if bytes.get(i) == Some(&b' ') {
                    i += 1;
                }
                end = i;
            }
            _ => break,
        }
    }
    (&line[..end], &line[end..])
}

/// The list marker at the start of `bare` (after indentation), with its
/// trailing space, such as "- " or "12. ".
fn list_marker(bare: &str) -> Option<&str> {
    for m in ["- [ ] ", "- [x] ", "* [ ] ", "* [x] ", "- ", "* ", "+ "] {
        if bare.starts_with(m) {
            return Some(&bare[..m.len()]);
        }
    }
    let digits = bare.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let rest = &bare[digits..];
        if rest.starts_with(". ") || rest.starts_with(") ") {
            return Some(&bare[..digits + 2]);
        }
    }
    None
}

fn wrap_block(lines: &[&str], width: usize) -> Vec<String> {
    let first = lines[0];
    let (quote, rest) = strip_quotes(first);
    let indent_len = rest.len() - rest.trim_start().len();
    let indent = &rest[..indent_len];
    let body = &rest[indent_len..];
    let marker = list_marker(body).unwrap_or("");

    let first_prefix = format!("{quote}{indent}{marker}");
    let next_prefix = format!("{quote}{indent}{}", " ".repeat(marker.chars().count()));
    let hard_break = lines
        .last()
        .is_some_and(|l| l.ends_with("  ") || l.ends_with('\\'));
    let trailer = lines
        .last()
        .map(|l| {
            if l.ends_with('\\') {
                "\\"
            } else if l.ends_with("  ") {
                "  "
            } else {
                ""
            }
        })
        .unwrap_or("");

    let mut words: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let text = if i == 0 {
            &body[marker.len()..]
        } else {
            strip_quotes(line).1
        };
        let text = if hard_break && i == lines.len() - 1 {
            text.trim_end_matches('\\')
        } else {
            text
        };
        words.extend(text.split_whitespace());
    }

    let mut out = Vec::new();
    let mut current = first_prefix.clone();
    let mut has_word = false;
    for word in words {
        let len = current.chars().count();
        if has_word && len + 1 + word.chars().count() > width {
            out.push(current);
            current = next_prefix.clone();
            has_word = false;
        }
        if has_word {
            current.push(' ');
        }
        current.push_str(word);
        has_word = true;
    }
    current.push_str(trailer);
    out.push(current);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_a_plain_paragraph() {
        let text = "one two three four five six seven eight nine ten eleven twelve";
        assert_eq!(
            reflow(text, 20).unwrap(),
            "one two three four\nfive six seven eight\nnine ten eleven\ntwelve"
        );
    }

    #[test]
    fn joins_short_lines() {
        assert_eq!(reflow("a\nb\nc", 80).unwrap(), "a b c");
        assert_eq!(reflow("a b c", 80), None);
    }

    #[test]
    fn keeps_list_markers_and_hangs_continuations() {
        let text = "- alpha beta gamma delta epsilon zeta eta\n- theta iota";
        assert_eq!(
            reflow(text, 22).unwrap(),
            "- alpha beta gamma\n  delta epsilon zeta\n  eta\n- theta iota"
        );
        let text = "10. alpha beta gamma delta";
        assert_eq!(reflow(text, 20).unwrap(), "10. alpha beta gamma\n    delta");
    }

    #[test]
    fn keeps_block_quotes() {
        let text = "> alpha beta gamma delta epsilon\n> zeta";
        assert_eq!(
            reflow(text, 20).unwrap(),
            "> alpha beta gamma\n> delta epsilon zeta"
        );
    }

    #[test]
    fn leaves_code_headings_and_tables_alone() {
        let text = "# A heading that is long enough to wrap\n\n```\nlet x = a very long line of code that must not wrap;\n```\n| a | b |";
        assert_eq!(reflow(text, 20), None);
    }

    #[test]
    fn keeps_hard_breaks() {
        let text = "first line  \nsecond line";
        assert_eq!(reflow(text, 80), None);
    }
}
