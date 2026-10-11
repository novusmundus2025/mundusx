use crate::contribution_contract::Operation;
use crate::media_runtime::memory::MediaBudget;
use crossterm::{
    cursor::MoveTo,
    event::{read, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
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
        "Images - Qwen Image (network queue needs node admission)",
        true,
    ),
    (
        Operation::TextToVideo,
        "Videos - Wan 14B (network queue needs node admission)",
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

    fn toggle(&mut self) -> String {
        // Previously saved future selections can be removed, but not newly enabled.
        if self.available(self.cursor) || self.selected[self.cursor] {
            self.selected[self.cursor] = !self.selected[self.cursor];
            return format!("{}: {}", if self.selected[self.cursor] { "Selected" } else { "Deselected" }, OPTIONS[self.cursor].1);
        }
        self.disabled_reason(self.cursor).unwrap_or_default()
    }

    fn disabled_reason(&self, index: usize) -> Option<String> {
        if !OPTIONS[index].2 {
            return Some("Unavailable in this release.".into());
        }
        self.budget.require_operation(OPTIONS[index].0).err()
    }

    fn space_key(&mut self, key: &KeyEvent) -> Option<String> {
        (key.code == KeyCode::Char(' ') && key.kind == KeyEventKind::Press)
            .then(|| self.toggle())
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
    enable_raw_mode().map_err(|e| format!("Cannot open workload selector: {e}"))?;
    let raw = RawMode;
    let mut selected = [current.contains(&Operation::Llm), current.iter().any(|op| *op != Operation::Llm)];
    let mut cursor = 0usize;
    let mut message = String::new();
    loop {
        let mut out = io::stdout();
        execute!(out, MoveTo(0, 0), Clear(ClearType::All)).map_err(|e| e.to_string())?;
        write!(out, "{}\r\n\r\n", crate::theme::menu_title("Choose contribution workloads")).map_err(|e| e.to_string())?;
        for (index, (label, detail)) in [
            ("LLM", "Chat and coding · managed model or existing local engine"),
            ("Media", "Images and video · choose media operations next"),
        ].iter().enumerate() {
            write!(out, "{} [{}] {}\r\n    {}\r\n\r\n",
                crate::theme::menu_marker(cursor == index), if selected[index] { "x" } else { " " },
                crate::theme::menu_label(label, cursor == index), crate::theme::muted(detail)).map_err(|e| e.to_string())?;
        }
        write!(out, "{}\r\n{}\r\n{}\r\n", crate::theme::hint("↑↓ Move · Space Toggle · Enter Continue · Esc Cancel"),
            crate::theme::muted("Choose one or both. Media availability is checked on the next screen."),
            crate::theme::muted(&message)).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        if let Event::Key(key) = read().map_err(|e| e.to_string())? {
            if key.kind != KeyEventKind::Press { continue; }
            match key.code {
                KeyCode::Esc => return Err("Workload selection cancelled; saved choices are unchanged.".into()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Err("Workload selection cancelled.".into()),
                KeyCode::Up | KeyCode::BackTab => cursor = 0,
                KeyCode::Down | KeyCode::Tab => cursor = 1,
                KeyCode::Char(' ') => selected[cursor] = !selected[cursor],
                KeyCode::Char('a' | 'A') => selected = [true, true],
                KeyCode::Enter if selected.iter().any(|v| *v) => {
                    drop(raw);
                    let mut operations = if selected[1] { prompt_media(current, budget)? } else { Vec::new() };
                    if selected[0] { operations.insert(0, Operation::Llm); }
                    return Ok(operations);
                }
                KeyCode::Enter => message = "Select LLM, Media, or both.".into(),
                _ => {}
            }
        }
    }
}

fn prompt_media(current: &[Operation], budget: &MediaBudget) -> Result<Vec<Operation>, String> {
    enable_raw_mode().map_err(|e| format!("Cannot open workload selector: {e}. Use --workloads llm,image,video for scripted setup."))?;
    let _raw = RawMode;
    let mut picker = Picker::new(current, budget.clone());
    picker.selected[0] = false;
    let visible: Vec<usize> = (1..OPTIONS.len()).filter(|i| OPTIONS[*i].2 || picker.selected[*i]).collect();
    picker.cursor = visible[0];
    let mut message = String::new();
    loop {
        let mut out = io::stdout();
        execute!(out, MoveTo(0, 0), Clear(ClearType::All)).map_err(|e| e.to_string())?;
        write!(out, "{}\r\n\r\n", crate::theme::menu_title("Media · choose image and video operations")).map_err(|e| e.to_string())?;
        writeln!(out, "{}\r", crate::theme::muted("Only media operations that fit your contribution limit can be enabled.")).map_err(|e| e.to_string())?;
        for &index in &visible {
            let label = match OPTIONS[index].0 {
                Operation::TextToImage => "Image generation · Qwen Image",
                Operation::TextToVideo => "Video generation · Wan",
                Operation::ImageEdit => "Image editing · saved selection (unavailable)",
                _ => "Image-to-video · saved selection (unavailable)",
            };
            write!(
                out,
                "{} [{}] {}{}\r\n",
                crate::theme::menu_marker(picker.cursor == index),
                if picker.selected[index] { "x" } else { " " },
                crate::theme::menu_label(label, picker.cursor == index),
                if !picker.available(index) {
                    " — disabled"
                } else {
                    ""
                }
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(reason) = picker.disabled_reason(picker.cursor) {
            write!(out, "\r\n{reason}\r\n").map_err(|e| e.to_string())?;
        }
        write!(out, "\r\nUp/Down or Tab: move | Space: toggle | A: all available | Enter: continue | Esc: cancel\r\nSelections do not start services or download models.\r\n{message}\r\n").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        if let Event::Key(key) = read().map_err(|e| e.to_string())? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if let Some(feedback) = picker.space_key(&key) {
                message = feedback;
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
                KeyCode::Up | KeyCode::BackTab => {
                    let position = visible.iter().position(|i| *i == picker.cursor).unwrap_or(0);
                    picker.cursor = visible[position.saturating_sub(1)];
                },
                KeyCode::Down | KeyCode::Tab => {
                    let position = visible.iter().position(|i| *i == picker.cursor).unwrap_or(0);
                    picker.cursor = visible[(position + 1).min(visible.len() - 1)];
                }
                KeyCode::Char(' ') => {}, // Ignore repeat events; toggle once per press.
                KeyCode::Char('a' | 'A') => {
                    picker.select_all();
                    picker.selected[0] = false;
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
    fn space_selects_and_deselects_once_per_press() {
        let mut picker = Picker::new(&[Operation::Llm], MediaBudget::new(Some(128 * 1024 * 1024 * 1024), 80));
        picker.cursor = 1;
        let press = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(picker.space_key(&press).unwrap().starts_with("Selected:"));
        assert!(picker.selected[1]);
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            let mut event = press;
            event.kind = kind;
            assert!(picker.space_key(&event).is_none());
            assert!(picker.selected[1]);
        }
        assert!(picker.space_key(&press).unwrap().starts_with("Deselected:"));
        assert!(!picker.selected[1]);
        assert_eq!(picker.result().unwrap(), vec![Operation::Llm]);
    }

    #[test]
    fn space_explains_model_specific_budget_and_unavailable_operations() {
        let mut picker = Picker::new(&[Operation::Llm], MediaBudget::new(Some(32 * 1024 * 1024 * 1024), 80));
        let press = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        for (index, required) in [(1, "32 GiB"), (2, "64 GiB")] {
            picker.cursor = index;
            let feedback = picker.space_key(&press).unwrap();
            assert!(feedback.contains(required));
            assert!(feedback.contains("25.60 GiB"));
            assert!(!picker.selected[index]);
        }
        picker.cursor = 3;
        assert_eq!(picker.space_key(&press).unwrap(), "Unavailable in this release.");
        assert!(!picker.selected[3]);
    }
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
