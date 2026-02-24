use crate::inject::context::AppContext;

pub fn build_system_prompt(ctx: &AppContext, personal_dict: &[String]) -> String {
    let dict_str = if personal_dict.is_empty() {
        "None".to_string()
    } else {
        personal_dict.join(", ")
    };

    let mut context_lines = format!("- App: {} ({})\n- Tone: {}\n- Personal vocab: {}",
        ctx.app_name, ctx.category, ctx.tone, dict_str);

    if !ctx.window_title.is_empty() {
        context_lines.push_str(&format!("\n- Window: {}", ctx.window_title));
    }
    if !ctx.selected_text.is_empty() {
        context_lines.push_str(&format!("\n- Nearby text: {}", ctx.selected_text));
    }

    format!(
r#"You are a dictation polisher. You receive raw speech transcripts and output ONLY the clean, polished text. Rules:
1. Remove ALL filler words (um, uh, like, you know, basically, actually, so)
2. Remove false starts and self-corrections — keep only the final intent
3. Fix grammar, spelling, punctuation, and capitalization
4. Handle voice commands: "new paragraph" → paragraph break, "new line" → line break
5. Match the tone specified below
6. Preserve technical terms and proper nouns exactly
7. Use the window title and nearby text to correct ambiguous words (e.g. technical terms, names, project-specific jargon)
8. Output ONLY the polished text. No explanations, no quotes.

CONTEXT:
{}

RAW TRANSCRIPT:
"#,
        context_lines
    )
}
