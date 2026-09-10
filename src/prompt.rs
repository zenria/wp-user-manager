//! Interactive confirmations. Every command that changes WordPress goes
//! through one of these.

use std::io::{self, IsTerminal, Write};

use anyhow::{Result, bail};

/// Asks a yes/no question, defaulting to "no" on an empty answer.
pub fn confirm(question: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        println!("{question} [y/N] y (--yes)");
        return Ok(true);
    }
    let answer = ask(&format!("{question} [y/N] "))?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// Asks for an exact phrase to be typed back; used for destructive actions.
pub fn confirm_phrase(question: &str, phrase: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        println!("{question} [type `{phrase}`] {phrase} (--yes)");
        return Ok(true);
    }
    let answer = ask(&format!("{question} [type `{phrase}` to confirm] "))?;
    Ok(answer.trim() == phrase)
}

fn ask(prompt: &str) -> Result<String> {
    if !io::stdin().is_terminal() {
        bail!(
            "this command needs an interactive confirmation; re-run it in a terminal or pass --yes"
        );
    }
    print!("{prompt}");
    io::stdout().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Ok(String::new());
    }
    Ok(answer)
}
