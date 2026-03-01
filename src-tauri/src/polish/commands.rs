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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_new_paragraph() {
        assert!(matches!(parse_command("new paragraph"), VoiceCommand::NewParagraph));
        assert!(matches!(parse_command("  New Paragraph  "), VoiceCommand::NewParagraph));
        assert!(matches!(parse_command("next paragraph"), VoiceCommand::NewParagraph));
    }

    #[test]
    fn parse_new_line() {
        assert!(matches!(parse_command("new line"), VoiceCommand::NewLine));
        assert!(matches!(parse_command("next line"), VoiceCommand::NewLine));
    }

    #[test]
    fn parse_punctuation() {
        assert!(matches!(parse_command("period"), VoiceCommand::Period));
        assert!(matches!(parse_command("full stop"), VoiceCommand::Period));
        assert!(matches!(parse_command("comma"), VoiceCommand::Comma));
        assert!(matches!(parse_command("question mark"), VoiceCommand::QuestionMark));
        assert!(matches!(parse_command("exclamation mark"), VoiceCommand::ExclamationMark));
        assert!(matches!(parse_command("exclamation point"), VoiceCommand::ExclamationMark));
    }

    #[test]
    fn parse_editing_commands() {
        assert!(matches!(parse_command("scratch that"), VoiceCommand::ScratchThat));
        assert!(matches!(parse_command("delete that"), VoiceCommand::ScratchThat));
        assert!(matches!(parse_command("undo"), VoiceCommand::Undo));
    }

    #[test]
    fn parse_normal_text_passthrough() {
        match parse_command("hello world") {
            VoiceCommand::None(t) => assert_eq!(t, "hello world"),
            _ => panic!("should be None variant"),
        }
    }

    #[test]
    fn command_text_values() {
        assert_eq!(command_text(&VoiceCommand::NewParagraph), Some("\n\n"));
        assert_eq!(command_text(&VoiceCommand::NewLine), Some("\n"));
        assert_eq!(command_text(&VoiceCommand::Period), Some("."));
        assert_eq!(command_text(&VoiceCommand::Comma), Some(","));
        assert_eq!(command_text(&VoiceCommand::QuestionMark), Some("?"));
        assert_eq!(command_text(&VoiceCommand::ExclamationMark), Some("!"));
        assert_eq!(command_text(&VoiceCommand::ScratchThat), None);
        assert_eq!(command_text(&VoiceCommand::Undo), None);
        assert_eq!(command_text(&VoiceCommand::None("hi".into())), None);
    }
}
