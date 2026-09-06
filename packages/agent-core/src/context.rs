#[derive(Clone, Debug, PartialEq)]
pub struct ContextWindow {
    pub text: String,
    pub original_chars: usize,
    pub compacted: bool,
}

pub fn compact_transcript(transcript: &str, max_chars: usize) -> ContextWindow {
    let max_chars = max_chars.max(1024);
    if transcript.len() <= max_chars {
        return ContextWindow {
            text: transcript.to_string(),
            original_chars: transcript.len(),
            compacted: false,
        };
    }
    let marker = "[Earlier conversation compacted]\n\n";
    let keep = max_chars.saturating_sub(marker.len());
    let mut start = transcript.len().saturating_sub(keep);
    while !transcript.is_char_boundary(start) {
        start += 1;
    }
    ContextWindow {
        text: format!("{marker}{}", &transcript[start..]),
        original_chars: transcript.len(),
        compacted: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_recent_context_with_a_visible_compaction_marker() {
        let input = format!("old{}recent", "x".repeat(3000));
        let output = compact_transcript(&input, 1024);
        assert!(output.compacted);
        assert!(output.text.starts_with("[Earlier conversation compacted]"));
        assert!(output.text.ends_with("recent"));
        assert!(output.text.len() <= 1024);
    }
}
