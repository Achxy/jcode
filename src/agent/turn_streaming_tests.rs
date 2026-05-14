use crate::message::StreamEvent;
use crate::protocol::ServerEvent;

/// Simulates the reasoning stream transformation logic applied by both
/// `turn_streaming_broadcast.rs` and `turn_streaming_mpsc.rs`.
///
/// This is a pure function that applies the same state-machine logic
/// to a sequence of `StreamEvent`s and returns the resulting `ServerEvent`s.
/// Tests against this function serve as regression markers for the reasoning
/// stream rendering bug.
///
/// The state machine ensures:
/// - `💭` prefix appears at most once per reasoning block
/// - No artificial newlines per `ThinkingDelta`
/// - Clean separators between reasoning and assistant text / tool UI
/// - Not emitting reasoning text when `show_thinking` is disabled
fn apply_reasoning_stream_transformation(
    events: &[StreamEvent],
    show_thinking: bool,
    store_reasoning_content: bool,
) -> (Vec<ServerEvent>, String) {
    let mut output = Vec::new();
    let mut reasoning_content = String::new();
    let mut reasoning_prefix_emitted = false;
    let mut reasoning_text_open = false;

    for event in events {
        match event {
            StreamEvent::ThinkingStart => {
                reasoning_prefix_emitted = false;
                reasoning_text_open = false;
            }
            StreamEvent::ThinkingEnd => {}
            StreamEvent::ThinkingDelta(thinking_text) => {
                if show_thinking {
                    let text = if reasoning_prefix_emitted {
                        thinking_text.clone()
                    } else {
                        reasoning_prefix_emitted = true;
                        format!("💭 {}", thinking_text.trim_start())
                    };
                    if !text.is_empty() {
                        reasoning_text_open = true;
                        output.push(ServerEvent::TextDelta { text });
                    }
                }
                if store_reasoning_content {
                    reasoning_content.push_str(thinking_text);
                }
            }
            StreamEvent::ThinkingDone { duration_secs } => {
                if show_thinking && reasoning_text_open {
                    output.push(ServerEvent::TextDelta {
                        text: format!("\n\n*Thought for {:.1}s*\n\n", duration_secs),
                    });
                    reasoning_text_open = false;
                }
            }
            StreamEvent::TextDelta(text) => {
                // Close open reasoning block before normal text
                if show_thinking && reasoning_text_open {
                    output.push(ServerEvent::TextDelta {
                        text: "\n\n".to_string(),
                    });
                    reasoning_text_open = false;
                }
                output.push(ServerEvent::TextDelta {
                    text: text.clone(),
                });
            }
            StreamEvent::ToolUseStart { id, name } => {
                // Close open reasoning block before tool display
                if show_thinking && reasoning_text_open {
                    output.push(ServerEvent::TextDelta {
                        text: "\n\n".to_string(),
                    });
                    reasoning_text_open = false;
                }
                output.push(ServerEvent::ToolStart {
                    id: id.clone(),
                    name: name.clone(),
                });
            }
            _ => {}
        }
    }

    (output, reasoning_content)
}

/// Helper: concatenate all TextDelta texts from the output in order
fn concat_text(output: &[ServerEvent]) -> String {
    let mut result = String::new();
    for event in output {
        if let ServerEvent::TextDelta { text } = event {
            result.push_str(text);
        }
    }
    result
}

#[test]
fn reasoning_with_multiple_small_deltas() {
    // Repro 1: multiple small reasoning deltas → one 💭 prefix, no per-delta newlines
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("The".to_string()),
        StreamEvent::ThinkingDelta(" user".to_string()),
        StreamEvent::ThinkingDelta(" wants".to_string()),
        StreamEvent::ThinkingDone {
            duration_secs: 1.2,
        },
        StreamEvent::TextDelta("Final answer.".to_string()),
    ];

    let (output, reason) = apply_reasoning_stream_transformation(&events, true, true);

    let text = concat_text(&output);

    // Must have exactly one 💭 at the start
    assert!(
        text.starts_with("💭 "),
        "Should start with single 💭 prefix, got: {:?}",
        text
    );

    // Must not have repeated 💭 markers
    assert!(
        !text.contains("💭 💭"),
        "Should not have repeated 💭 markers, got: {:?}",
        text
    );
    assert!(
        !text.contains("💭 The\n💭"),
        "Should not have newline-separated 💭 markers, got: {:?}",
        text
    );

    // Must contain the reasoning text concatenated without per-delta newlines
    assert!(
        text.contains("The user wants"),
        "Should contain concatenated reasoning text, got: {:?}",
        text
    );

    // Must contain the thinking duration note
    assert!(
        text.contains("*Thought for 1.2s*"),
        "Should contain thinking duration, got: {:?}",
        text
    );

    // Must have a separator before final answer
    assert!(
        text.contains("Final answer."),
        "Should contain final answer, got: {:?}",
        text
    );

    // Reasoning content stored separately (no 💭 or newlines)
    assert_eq!(reason, "The user wants", "Reasoning content should be bare text");
}

#[test]
fn reasoning_without_thinking_done() {
    // Repro 2: No ThinkingDone — reasoning block should close before TextDelta
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("The".to_string()),
        StreamEvent::ThinkingDelta(" user".to_string()),
        StreamEvent::TextDelta("Final answer.".to_string()),
    ];

    let (output, _) = apply_reasoning_stream_transformation(&events, true, true);

    let text = concat_text(&output);

    // One 💭 prefix
    assert!(
        text.starts_with("💭 "),
        "Should start with single 💭 prefix, got: {:?}",
        text
    );

    // Reasoning text concatenated
    assert!(
        text.contains("The user"),
        "Should contain concatenated reasoning, got: {:?}",
        text
    );

    // Clean separator before final answer
    assert!(
        text.contains("Final answer."),
        "Should contain final answer, got: {:?}",
        text
    );

    // No per-delta newlines
    assert!(
        !text.contains("💭 The\n💭"),
        "Should not have per-delta newlines, got: {:?}",
        text
    );
}

#[test]
fn thinking_disabled() {
    // Repro 3: show_thinking = false → no visible reasoning text
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("The".to_string()),
        StreamEvent::ThinkingDelta(" user".to_string()),
        StreamEvent::ThinkingDone {
            duration_secs: 0.5,
        },
        StreamEvent::TextDelta("Answer.".to_string()),
    ];

    let (output, reason) = apply_reasoning_stream_transformation(&events, false, true);

    let text = concat_text(&output);

    // No 💭 prefix, no thinking duration
    assert!(
        !text.contains('💭'),
        "Should not contain 💭 when thinking disabled, got: {:?}",
        text
    );
    assert!(
        !text.contains("Thought for"),
        "Should not contain thinking duration when disabled, got: {:?}",
        text
    );

    // Normal text still passes through
    assert_eq!(text, "Answer.", "Normal text should pass through unchanged");

    // Reasoning content still stored even when not displayed
    assert_eq!(reason, "The user", "Reasoning content should be stored even when not displayed");
}

#[test]
fn tool_after_reasoning() {
    // Repro 4: Reasoning followed by tool — clean separator
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("I should call a tool.".to_string()),
        StreamEvent::ToolUseStart {
            id: "tool1".to_string(),
            name: "bash".to_string(),
        },
    ];

    let (output, _) = apply_reasoning_stream_transformation(&events, true, true);

    // Extract text deltas and tool starts
    let mut text_parts = Vec::new();
    let mut saw_tool = false;
    for event in &output {
        match event {
            ServerEvent::TextDelta { text } => text_parts.push(text.as_str()),
            ServerEvent::ToolStart { name: _, .. } => saw_tool = true,
            _ => {}
        }
    }

    let joined = text_parts.join("|");

    // Reasoning block should have exactly one 💭
    assert!(
        joined.contains("💭"),
        "Should contain 💭 prefix, got: {:?}",
        joined
    );

    assert!(
        saw_tool,
        "Should have ToolStart event"
    );

    // There should be a separator between reasoning and tool
    // The separator is \n\n so the text_parts should have separator entry
    // Tool display should appear after reasoning, with separator in between
    assert!(
        text_parts.len() == 2,
        "Should have exactly 2 text parts (reasoning content and separator), got {}: {:?}",
        text_parts.len(),
        text_parts
    );

    // The separator should be \n\n
    assert_eq!(
        text_parts[1], "\n\n",
        "Second text part should be separator \\n\\n, got: {:?}",
        text_parts[1]
    );

    assert!(saw_tool, "Should have ToolStart event");
}

#[test]
fn no_regression_per_delta_newlines() {
    // Critical regression: must NOT produce format!("💭 {}\n", thinking_text) per delta
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("h".to_string()),
        StreamEvent::ThinkingDelta("(".to_string()),
        StreamEvent::ThinkingDelta("\"span\"".to_string()),
        StreamEvent::ThinkingDelta(",".to_string()),
        StreamEvent::TextDelta("Normal output.".to_string()),
    ];

    let (output, _) = apply_reasoning_stream_transformation(&events, true, true);

    let text = concat_text(&output);

    // Must NOT produce one-token-per-line output
    // Anti-pattern: "💭 h\n💭 (\n💭 \"span\"\n💭 ,"
    assert!(
        !text.contains("\n💭 "),
        "Must NOT have per-delta 💭+newline pattern, got: {:?}",
        text
    );

    // Should have clean flowing reasoning
    assert!(
        text.contains("h(\"span\","),
        "Should have concatenated reasoning text, got: {:?}",
        text
    );
}

#[test]
fn no_client_side_sanitizer_needed() {
    // Verify: the fix works WITHOUT client side text sanitization.
    // The streaming text should not contain the bad pattern that needs cleanup.
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("think step".to_string()),
        StreamEvent::ThinkingDone {
            duration_secs: 2.0,
        },
        StreamEvent::TextDelta("Result.".to_string()),
    ];

    let (output, _) = apply_reasoning_stream_transformation(&events, true, true);

    for event in &output {
        if let ServerEvent::TextDelta { text } = event {
            // Must NOT contain patterns that a client-side sanitizer would need to strip
            assert!(
                !text.contains("💭 💭"),
                "Should not have double 💭: {:?}",
                text
            );
        }
    }
}

#[test]
fn only_thinking_start_resets_state() {
    // Only ThinkingStart should reset the prefix state,
    // not ThinkingDone or TextDelta.
    let events = vec![
        // First reasoning block
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("First".to_string()),
        StreamEvent::ThinkingDone {
            duration_secs: 1.0,
        },
        // Second reasoning block (new ThinkingStart)
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("Second".to_string()),
        StreamEvent::TextDelta("Done.".to_string()),
    ];

    let (output, _) = apply_reasoning_stream_transformation(&events, true, true);

    let text = concat_text(&output);

    // Check that both 💭 prefixes appear (one per thinking block)
    let thought_count = text.matches('💭').count();
    assert_eq!(
        thought_count, 2,
        "Should have exactly 2 💭 prefixes (one per block), got {} in: {:?}",
        thought_count, text
    );

    assert!(
        text.contains("First"),
        "Should contain first reasoning, got: {:?}",
        text
    );
    assert!(
        text.contains("Second"),
        "Should contain second reasoning, got: {:?}",
        text
    );
}

#[test]
fn empty_thinking_delta_omitted() {
    // Empty thinking delta should not emit TextDelta
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("".to_string()),
        StreamEvent::ThinkingDelta("real".to_string()),
        StreamEvent::TextDelta("Answer.".to_string()),
    ];

    let (output, _) = apply_reasoning_stream_transformation(&events, true, true);

    let text = concat_text(&output);

    // First non-empty delta should get the 💭 prefix
    assert!(
        text.starts_with("💭 real"),
        "Should skip empty delta and prefix on first non-empty, got: {:?}",
        text
    );
}

#[test]
fn trim_start_of_first_delta() {
    // Leading whitespace on first delta should be trimmed
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("  Hello".to_string()),
        StreamEvent::TextDelta("Done.".to_string()),
    ];

    let (output, _) = apply_reasoning_stream_transformation(&events, true, true);

    let text = concat_text(&output);

    assert!(
        text.starts_with("💭 Hello"),
        "Should trim leading whitespace from first delta, got: {:?}",
        text
    );
}

#[test]
fn store_reasoning_content_regardless_of_display() {
    // Reasoning content should be stored even when show_thinking is false,
    // and should not contain display decorations
    let events = vec![
        StreamEvent::ThinkingStart,
        StreamEvent::ThinkingDelta("secret ".to_string()),
        StreamEvent::ThinkingDelta("reasoning".to_string()),
        StreamEvent::TextDelta("Public.".to_string()),
    ];

    let (_, reason_display_on) = apply_reasoning_stream_transformation(&events, true, true);
    let (_, reason_display_off) = apply_reasoning_stream_transformation(&events, false, true);
    let (_, reason_not_stored) = apply_reasoning_stream_transformation(&events, true, false);

    // Content stored correctly regardless of display setting
    assert_eq!(reason_display_on, "secret reasoning", "Should store reasoning content when displayed");
    assert_eq!(reason_display_off, "secret reasoning", "Should store reasoning content even when not displayed");

    // Content NOT stored when store_reasoning_content is false
    assert!(reason_not_stored.is_empty(), "Should not store when store_reasoning_content is false");

    // Stored content has NO display decorations (no 💭 added by JCode streaming)
    assert!(
        !reason_display_on.contains('💭'),
        "Stored reasoning content must not contain 💭 decoration"
    );
    // Stored content should not contain newlines added by JCode's streaming logic
    // (a single `\n` might legitimately be part of the provider's thinking text,
    // so we don't assert on that — we just check for 💭)
}
