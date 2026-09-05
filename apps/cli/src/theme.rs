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
    #[value(alias = "reactor")]
    Mundusx,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThemeMode {
    Classic,
    Mundusx,
}

impl ThemeMode {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Mundusx,
            _ => Self::Classic,
        }
    }
}

pub fn configure(selection: ThemeSelection) {
    let mode = match selection {
        ThemeSelection::Classic => ThemeMode::Classic,
        ThemeSelection::Mundusx if rich_output_allowed() => ThemeMode::Mundusx,
        ThemeSelection::Mundusx => ThemeMode::Classic,
        ThemeSelection::Auto if rich_output_allowed() => ThemeMode::Mundusx,
        ThemeSelection::Auto => ThemeMode::Classic,
    };

    ACTIVE_THEME.store(
        match mode {
            ThemeMode::Classic => 0,
            ThemeMode::Mundusx => 1,
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
        ThemeMode::Mundusx => println!("{}", style(title).with(brand()).bold()),
    }
}

pub fn field(label: &str, value: impl std::fmt::Display) {
    match current_mode() {
        ThemeMode::Classic => println!("{label}: {value}"),
        ThemeMode::Mundusx => println!(
            "{} {}",
            style(format!("{label}:")).with(Color::DarkGrey),
            value
        ),
    }
}

pub fn note(value: impl std::fmt::Display) {
    match current_mode() {
        ThemeMode::Classic => println!("note: {value}"),
        ThemeMode::Mundusx => println!("{} {value}", style("•").with(brand())),
    }
}

pub fn warn(value: impl std::fmt::Display) {
    match current_mode() {
        ThemeMode::Classic => eprintln!("warning: {value}"),
        ThemeMode::Mundusx => eprintln!("{} {value}", style("!").with(Color::Yellow).bold()),
    }
}

pub fn error(value: impl std::fmt::Display) {
    match current_mode() {
        ThemeMode::Classic => eprintln!("error: {value}"),
        ThemeMode::Mundusx => eprintln!(
            "{} {}",
            style("×").with(Color::Red).bold(),
            style(value.to_string()).with(Color::Red).bold()
        ),
    }
}

pub fn status(value: &str) -> String {
    match current_mode() {
        ThemeMode::Classic => value.to_string(),
        ThemeMode::Mundusx => {
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
        ThemeMode::Mundusx => {
            let top = format!("╭{}╮", "─".repeat(width + 4));
            let bottom = format!("╰{}╯", "─".repeat(width + 4));
            println!("{}", style(top).with(Color::DarkGrey));
            println!(
                "{} {} {}",
                style("│").with(Color::DarkGrey),
                style(format!("{:<width$}", title, width = width + 2))
                    .with(accent)
                    .bold(),
                style("│").with(Color::DarkGrey)
            );
            println!(
                "{} {} {}",
                style("│").with(Color::DarkGrey),
                style(format!("{:<width$}", subtitle, width = width + 2)).with(Color::DarkGrey),
                style("│").with(Color::DarkGrey)
            );
            println!(
                "{}",
                style(format!("├{}┤", "─".repeat(width + 4))).with(Color::DarkGrey)
            );
            for line in lines {
                println!(
                    "{} {:<width$} {}",
                    style("│").with(Color::DarkGrey),
                    line,
                    style("│").with(Color::DarkGrey),
                    width = width + 2
                );
            }
            println!("{}", style(bottom).with(Color::DarkGrey));
        }
    }
}

fn brand() -> Color {
    Color::AnsiValue(99)
}

pub fn banner(title: &str, subtitle: &str) {
    match current_mode() {
        ThemeMode::Classic => {
            println!("{title}");
            println!("{subtitle}");
        }
        ThemeMode::Mundusx => {
            println!();
            println!("{}", style("◆  MUNDUSX").with(brand()).bold());
            println!("{}", style(title).with(Color::White).bold());
            println!("{}", style(subtitle).with(Color::DarkGrey));
            println!();
        }
    }
}

pub fn menu_title(value: &str) -> String {
    match current_mode() {
        ThemeMode::Classic => value.to_string(),
        ThemeMode::Mundusx => style(value).with(Color::White).bold().to_string(),
    }
}

pub fn menu_rule() -> String {
    match current_mode() {
        ThemeMode::Classic => "------------------------------".to_string(),
        ThemeMode::Mundusx => style("──────────────────────────────")
            .with(Color::DarkGrey)
            .to_string(),
    }
}

pub fn menu_marker(selected: bool) -> String {
    match (current_mode(), selected) {
        (ThemeMode::Classic, true) => ">>".to_string(),
        (ThemeMode::Classic, false) => "  ".to_string(),
        // ASCII remains legible in legacy Windows PowerShell hosts whose
        // selected console font does not contain the heavier arrow glyphs.
        (ThemeMode::Mundusx, true) => style(">>").with(brand()).bold().to_string(),
        (ThemeMode::Mundusx, false) => "  ".to_string(),
    }
}

pub fn menu_label(value: impl std::fmt::Display, selected: bool) -> String {
    let value = value.to_string();
    match (current_mode(), selected) {
        (ThemeMode::Mundusx, true) => style(value).with(brand()).bold().to_string(),
        _ => value,
    }
}

pub fn muted(value: impl std::fmt::Display) -> String {
    let value = value.to_string();
    match current_mode() {
        ThemeMode::Classic => value,
        ThemeMode::Mundusx => style(value).with(Color::DarkGrey).to_string(),
    }
}

pub fn hint(value: impl std::fmt::Display) -> String {
    let value = value.to_string();
    match current_mode() {
        ThemeMode::Classic => value,
        ThemeMode::Mundusx => format!(
            "{} {}",
            style("->").with(brand()),
            style(value).with(Color::DarkGrey)
        ),
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
    fn no_color_forces_mundusx_selection_to_classic() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("NO_COLOR", "1");
        configure(ThemeSelection::Mundusx);

        assert_eq!(status("failed"), "failed");

        std::env::remove_var("NO_COLOR");
    }
}
