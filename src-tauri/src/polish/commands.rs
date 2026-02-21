/// Voice command parser. Detects commands in raw ASR output before LLM polish.
pub enum VoiceCommand {
    NewParagraph,
    NewLine,
    Period,
    Comma,
    QuestionMark,
    ExclamationMark,
    ScratchThat,
    Undo,
    None(String), // Not a command — pass through to LLM
}

pub fn parse_command(text: &str) -> VoiceCommand {
    let lower = text.trim().to_lowercase();
    match lower.as_str() {
        "new paragraph" | "next paragraph" => VoiceCommand::NewParagraph,
        "new line" | "next line" => VoiceCommand::NewLine,
        "period" | "full stop" => VoiceCommand::Period,
        "comma" => VoiceCommand::Comma,
        "question mark" => VoiceCommand::QuestionMark,
        "exclamation mark" | "exclamation point" => VoiceCommand::ExclamationMark,
        "delete that" | "scratch that" => VoiceCommand::ScratchThat,
        "undo" => VoiceCommand::Undo,
        _ => VoiceCommand::None(text.to_string()),
    }
}

/// Returns the text to inject for simple punctuation/formatting commands.
pub fn command_text(cmd: &VoiceCommand) -> Option<&'static str> {
    match cmd {
        VoiceCommand::NewParagraph => Some("\n\n"),
        VoiceCommand::NewLine => Some("\n"),
        VoiceCommand::Period => Some("."),
        VoiceCommand::Comma => Some(","),
        VoiceCommand::QuestionMark => Some("?"),
        VoiceCommand::ExclamationMark => Some("!"),
        _ => None,
    }
}
