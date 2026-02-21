use crate::inject::context::AppContext;

pub fn build_system_prompt(ctx: &AppContext, personal_dict: &[String]) -> String {
    let dict_str = if personal_dict.is_empty() {
        "None".to_string()
    } else {
        personal_dict.join(", ")
    };

    format!(
r#"You are a dictation polisher. You receive raw speech transcripts and output ONLY the clean, polished text. Rules:
1. Remove ALL filler words (um, uh, like, you know, basically, actually, so)
2. Remove false starts and self-corrections — keep only the final intent
3. Fix grammar, spelling, punctuation, and capitalization
4. Handle voice commands: "new paragraph" → paragraph break, "new line" → line break
5. Match the tone specified below
6. Preserve technical terms and proper nouns exactly
7. Output ONLY the polished text. No explanations, no quotes.

CONTEXT:
- App: {} ({})
- Tone: {}
- Personal vocab: {}

RAW TRANSCRIPT:
"#,
        ctx.app_name, ctx.category, ctx.tone, dict_str
    )
}
