use clap::ValueEnum;
use crossterm::style::{style, Color, Stylize};
use std::env;
use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicU8, Ordering};

static ACTIVE_THEME: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ThemeSelection {
    Auto,
    Classic,
    Reactor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThemeMode {
    Classic,
    Reactor,
}

impl ThemeMode {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Reactor,
            _ => Self::Classic,
        }
    }
}

pub fn configure(selection: ThemeSelection) {
    let mode = match selection {
        ThemeSelection::Classic => ThemeMode::Classic,
        ThemeSelection::Reactor if rich_output_allowed() => ThemeMode::Reactor,
        ThemeSelection::Reactor => ThemeMode::Classic,
        ThemeSelection::Auto if rich_output_allowed() => ThemeMode::Reactor,
        ThemeSelection::Auto => ThemeMode::Classic,
    };

    ACTIVE_THEME.store(
        match mode {
            ThemeMode::Classic => 0,
            ThemeMode::Reactor => 1,
        },
        Ordering::Relaxed,
    );
}

fn current_mode() -> ThemeMode {
    ThemeMode::from_u8(ACTIVE_THEME.load(Ordering::Relaxed))
}

fn rich_output_allowed() -> bool {
    env::var_os("NO_COLOR").is_none() && env::var_os("CI").is_none() && io::stdout().is_terminal()
}

pub fn section(title: &str) {
    match current_mode() {
        ThemeMode::Classic => println!("[{}]", title),
        ThemeMode::Reactor => println!(
            "{}",
            style(format!("== {} ==", title)).with(Color::Red).bold()
        ),
    }
}

pub fn field(label: &str, value: impl std::fmt::Display) {
    match current_mode() {
        ThemeMode::Classic => println!("{label}: {value}"),
        ThemeMode::Reactor => println!(
            "{} {}",
            style(format!("{label}:"))
                .with(Color::AnsiValue(220))
                .bold(),
            value
        ),
    }
}

pub fn note(value: impl std::fmt::Display) {
    match current_mode() {
        ThemeMode::Classic => println!("note: {value}"),
        ThemeMode::Reactor => println!("{} {value}", style("note:").with(Color::Cyan)),
    }
}

pub fn warn(value: impl std::fmt::Display) {
    match current_mode() {
        ThemeMode::Classic => eprintln!("warning: {value}"),
        ThemeMode::Reactor => eprintln!("{} {value}", style("warning:").with(Color::Yellow).bold()),
    }
}

pub fn error(value: impl std::fmt::Display) {
    match current_mode() {
        ThemeMode::Classic => eprintln!("error: {value}"),
        ThemeMode::Reactor => eprintln!(
            "{}",
            style(format!("error: {value}")).with(Color::Red).bold()
        ),
    }
}

pub fn status(value: &str) -> String {
    match current_mode() {
        ThemeMode::Classic => value.to_string(),
        ThemeMode::Reactor => {
            let color = match value.to_ascii_lowercase().as_str() {
                "completed" | "ready" | "yes" | "allowed" | "saved" => Color::Green,
                "failed" | "blocked" | "no" | "cancelled" | "canceled" => Color::Red,
                "queued" | "assigned" | "running" | "pending" | "review needed" => Color::Yellow,
                _ => Color::Cyan,
            };
            style(value).with(color).to_string()
        }
    }
}

pub fn boolean(value: bool, yes: &str, no: &str) -> String {
    if value {
        status(yes)
    } else {
        status(no)
    }
}

pub fn panel(title: &str, subtitle: &str, lines: &[String], accent: Color) {
    let mut width = title.chars().count().max(subtitle.chars().count());
    for line in lines {
        width = width.max(line.chars().count());
    }
    let width = width.max(24);
    let border = "-".repeat(width + 4);

    match current_mode() {
        ThemeMode::Classic => {
            println!("+{border}+");
            println!(
                "| {:<width$} |",
                title.to_ascii_uppercase(),
                width = width + 2
            );
            println!("| {:<width$} |", subtitle, width = width + 2);
            println!("+{border}+");
            for line in lines {
                println!("| {:<width$} |", line, width = width + 2);
            }
            println!("+{border}+");
        }
        ThemeMode::Reactor => {
            println!("{}", style(format!("+{border}+")).with(Color::DarkGrey));
            println!(
                "{}",
                style(format!(
                    "| {:<width$} |",
                    title.to_ascii_uppercase(),
                    width = width + 2
                ))
                .with(accent)
                .bold()
            );
            println!(
                "{}",
                style(format!("| {:<width$} |", subtitle, width = width + 2)).with(Color::DarkGrey)
            );
            println!("{}", style(format!("+{border}+")).with(Color::DarkGrey));
            for line in lines {
                println!("| {:<width$} |", line, width = width + 2);
            }
            println!("{}", style(format!("+{border}+")).with(Color::DarkGrey));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{boolean, configure, status, ThemeSelection};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn classic_theme_keeps_plain_status_text() {
        configure(ThemeSelection::Classic);

        assert_eq!(status("completed"), "completed");
        assert_eq!(boolean(false, "yes", "no"), "no");
    }

    #[test]
    fn no_color_forces_reactor_selection_to_classic() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("NO_COLOR", "1");
        configure(ThemeSelection::Reactor);

        assert_eq!(status("failed"), "failed");

        std::env::remove_var("NO_COLOR");
    }
}
