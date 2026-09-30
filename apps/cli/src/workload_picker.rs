use crate::contribution_contract::Operation;
use crate::media_runtime::memory::MediaBudget;
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
    budget: MediaBudget,
}

impl Picker {
    fn new(current: &[Operation], budget: MediaBudget) -> Self {
        Self {
            selected: OPTIONS.map(|(operation, _, _)| current.contains(&operation)),
            cursor: 0,
            budget,
        }
    }

    fn toggle(&mut self) {
        // Previously saved future selections can be removed, but not newly enabled.
        if self.available(self.cursor) || self.selected[self.cursor] {
            self.selected[self.cursor] = !self.selected[self.cursor];
        }
    }

    fn available(&self, index: usize) -> bool {
        OPTIONS[index].2 && self.budget.allows(OPTIONS[index].0)
    }

    fn select_all(&mut self) {
        for index in 0..OPTIONS.len() {
            if self.available(index) {
                self.selected[index] = true;
            }
        }
    }

    fn result(&self) -> Result<Vec<Operation>, String> {
        for (selected, (operation, _, _)) in self.selected.iter().zip(OPTIONS) {
            if *selected {
                self.budget.require_operation(operation)?;
            }
        }
        if !self
            .selected
            .iter()
            .zip(OPTIONS)
            .any(|(selected, (_, _, available))| *selected && available)
        {
            return Err("Select at least one available workload.".into());
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

pub fn prompt(current: &[Operation], budget: &MediaBudget) -> Result<Vec<Operation>, String> {
    enable_raw_mode().map_err(|e| format!("Cannot open workload selector: {e}. Use --workloads llm,image,video for scripted setup."))?;
    let _raw = RawMode;
    let mut picker = Picker::new(current, budget.clone());
    let mut message = String::new();
    loop {
        let mut out = io::stdout();
        execute!(out, MoveTo(0, 0), Clear(ClearType::All)).map_err(|e| e.to_string())?;
        write!(out, "Choose contribution workloads\r\n\r\n").map_err(|e| e.to_string())?;
        writeln!(out, "{}\r", budget.description()).map_err(|e| e.to_string())?;
        for (index, (operation, label, _)) in OPTIONS.iter().enumerate() {
            write!(
                out,
                "{} [{}] {}{}\r\n",
                if picker.cursor == index { ">" } else { " " },
                if picker.selected[index] { "x" } else { " " },
                label,
                if !operation.is_llm() && !budget.eligible {
                    " — unavailable: contributed memory below 24 GiB or unknown"
                } else {
                    ""
                }
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
                    picker.select_all();
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
    fn select_all_only_enables_models_that_fit_the_capped_budget() {
        let mut picker = Picker::new(
            &[Operation::Llm],
            MediaBudget::new(Some(64 * 1024 * 1024 * 1024), 50),
        );
        picker.cursor = 2;
        picker.toggle();
        picker.select_all();
        assert_eq!(
            picker.result().unwrap(),
            vec![Operation::Llm, Operation::TextToImage]
        );
        let saved = Picker::new(
            &[Operation::Llm, Operation::TextToVideo],
            picker.budget.clone(),
        );
        assert!(saved.result().unwrap_err().contains("64 GiB"));
        let mut big = Picker::new(
            &[Operation::Llm],
            MediaBudget::new(Some(128 * 1024 * 1024 * 1024), 50),
        );
        big.select_all();
        assert_eq!(
            big.result().unwrap(),
            vec![
                Operation::Llm,
                Operation::TextToImage,
                Operation::TextToVideo
            ]
        );
    }
    #[test]
    fn small_budget_disables_media_for_toggle_and_select_all() {
        let mut picker = Picker::new(
            &[Operation::Llm],
            MediaBudget::new(Some(32 * 1024 * 1024 * 1024), 50),
        );
        for index in 1..OPTIONS.len() {
            picker.cursor = index;
            picker.toggle();
        }
        picker.select_all();
        assert_eq!(picker.result().unwrap(), vec![Operation::Llm]);
        let mut saved = Picker::new(
            &[Operation::Llm, Operation::TextToImage],
            MediaBudget::new(Some(32 * 1024 * 1024 * 1024), 50),
        );
        assert!(saved.result().is_err());
        saved.cursor = 1;
        saved.toggle();
        assert_eq!(saved.result().unwrap(), vec![Operation::Llm]);
    }
    #[test]
    fn preserves_existing_selections_until_changed() {
        let p = Picker::new(
            &[Operation::Llm, Operation::TextToVideo, Operation::ImageEdit],
            MediaBudget::new(Some(128 * 1024 * 1024 * 1024), 80),
        );
        assert_eq!(
            p.result().unwrap(),
            vec![Operation::Llm, Operation::TextToVideo, Operation::ImageEdit]
        );
    }
    #[test]
    fn cannot_enable_unimplemented_operations() {
        let mut p = Picker::new(
            &[Operation::Llm],
            MediaBudget::new(Some(128 * 1024 * 1024 * 1024), 80),
        );
        p.cursor = 3;
        p.toggle();
        p.cursor = 4;
        p.toggle();
        assert_eq!(p.result().unwrap(), vec![Operation::Llm]);
    }
    #[test]
    fn allows_media_only_but_rejects_empty_selection() {
        let mut p = Picker::new(
            &[Operation::Llm],
            MediaBudget::new(Some(128 * 1024 * 1024 * 1024), 80),
        );
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
        let result = prompt(
            &[Operation::Llm],
            &MediaBudget::new(Some(128 * 1024 * 1024 * 1024), 80),
        );
        println!("PICKER_RESULT={result:?}");
    }
}
