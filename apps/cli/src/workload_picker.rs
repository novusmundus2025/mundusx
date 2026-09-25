use crate::contribution_contract::Operation;
use crossterm::{
    cursor::MoveTo,
    event::{read, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use std::io::{self, Write};

const OPTIONS: [(Operation, &str, bool); 5] = [
    (
        Operation::Llm,
        "Chat / coding - existing cluster or local model",
        true,
    ),
    (
        Operation::TextToImage,
        "Images - Qwen Image (local generation; network queue pending)",
        true,
    ),
    (
        Operation::TextToVideo,
        "Videos - Wan 14B (network queue needs LLM admission)",
        true,
    ),
    (
        Operation::ImageEdit,
        "Image editing - unavailable in this release",
        false,
    ),
    (
        Operation::ImageToVideo,
        "Image-to-video - unavailable in this release",
        false,
    ),
];

struct Picker {
    selected: [bool; 5],
    cursor: usize,
}

impl Picker {
    fn new(current: &[Operation]) -> Self {
        Self {
            selected: OPTIONS.map(|(operation, _, _)| current.contains(&operation)),
            cursor: 0,
        }
    }

    fn toggle(&mut self) {
        // Previously saved future selections can be removed, but not newly enabled.
        if OPTIONS[self.cursor].2 || self.selected[self.cursor] {
            self.selected[self.cursor] = !self.selected[self.cursor];
        }
    }

    fn result(&self) -> Result<Vec<Operation>, &'static str> {
        if !self
            .selected
            .iter()
            .zip(OPTIONS)
            .any(|(selected, (_, _, available))| *selected && available)
        {
            return Err("Select at least one available workload.");
        }
        Ok(OPTIONS
            .iter()
            .zip(self.selected)
            .filter_map(|((operation, _, _), selected)| selected.then_some(*operation))
            .collect())
    }
}

struct RawMode;
impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

pub fn prompt(current: &[Operation]) -> Result<Vec<Operation>, String> {
    enable_raw_mode().map_err(|e| format!("Cannot open workload selector: {e}. Use --workloads llm,image,video for scripted setup."))?;
    let _raw = RawMode;
    let mut picker = Picker::new(current);
    let mut message = "";
    loop {
        let mut out = io::stdout();
        execute!(out, MoveTo(0, 0), Clear(ClearType::All)).map_err(|e| e.to_string())?;
        write!(out, "Choose contribution workloads\r\n\r\n").map_err(|e| e.to_string())?;
        for (index, (_, label, _)) in OPTIONS.iter().enumerate() {
            write!(
                out,
                "{} [{}] {}\r\n",
                if picker.cursor == index { ">" } else { " " },
                if picker.selected[index] { "x" } else { " " },
                label
            )
            .map_err(|e| e.to_string())?;
        }
        write!(out, "\r\nUp/Down or Tab: move | Space: toggle | A: all available | Enter: continue | Esc: cancel\r\nSelections do not start services or download models.\r\n{message}\r\n").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        if let Event::Key(key) = read().map_err(|e| e.to_string())? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match key.code {
                KeyCode::Esc => {
                    return Err(
                        "Workload selection cancelled; saved workload choices are unchanged."
                            .into(),
                    )
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("Workload selection cancelled.".into())
                }
                KeyCode::Up | KeyCode::BackTab => picker.cursor = picker.cursor.saturating_sub(1),
                KeyCode::Down | KeyCode::Tab => {
                    picker.cursor = (picker.cursor + 1).min(OPTIONS.len() - 1)
                }
                KeyCode::Char(' ') => picker.toggle(),
                KeyCode::Char('a' | 'A') => {
                    for (index, (_, _, available)) in OPTIONS.iter().enumerate() {
                        if *available {
                            picker.selected[index] = true;
                        }
                    }
                }
                KeyCode::Enter => match picker.result() {
                    Ok(result) => return Ok(result),
                    Err(error) => message = error,
                },
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_existing_selections_until_changed() {
        let p = Picker::new(&[Operation::Llm, Operation::TextToVideo, Operation::ImageEdit]);
        assert_eq!(
            p.result().unwrap(),
            vec![Operation::Llm, Operation::TextToVideo, Operation::ImageEdit]
        );
    }
    #[test]
    fn cannot_enable_unimplemented_operations() {
        let mut p = Picker::new(&[Operation::Llm]);
        p.cursor = 3;
        p.toggle();
        p.cursor = 4;
        p.toggle();
        assert_eq!(p.result().unwrap(), vec![Operation::Llm]);
    }
    #[test]
    fn allows_media_only_but_rejects_empty_selection() {
        let mut p = Picker::new(&[Operation::Llm]);
        p.toggle();
        assert!(p.result().is_err());
        p.cursor = 1;
        p.toggle();
        p.cursor = 2;
        p.toggle();
        assert_eq!(
            p.result().unwrap(),
            vec![Operation::TextToImage, Operation::TextToVideo]
        );
    }
    #[test]
    #[ignore = "requires an interactive terminal; exercised by PTY smoke test"]
    fn terminal_smoke() {
        let result = prompt(&[Operation::Llm]);
        println!("PICKER_RESULT={result:?}");
    }
}
