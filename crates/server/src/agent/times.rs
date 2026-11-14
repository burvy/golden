//! The model may only repeat clock times our tools computed. Pure helpers for that check.

/// Every clock time ("19:58", "7:58") in `text` must be one of `allowed` (as "HH:MM").
pub fn times_ok(text: &str, allowed: &[String]) -> bool {
    clock_times(text).iter().all(|t| allowed.iter().any(|a| a == t || twelve_hour(a).as_deref() == Some(t.as_str())))
}

/// "19:58" -> "7:58"
fn twelve_hour(hhmm: &str) -> Option<String> {
    let (h, m) = hhmm.split_once(':')?;
    let h: u32 = h.parse().ok()?;
    Some(format!("{}:{m}", if h % 12 == 0 { 12 } else { h % 12 }))
}

/// Every `H:MM` / `HH:MM` in the text, as written. Not "16:9" (one digit after) or "123:45" (three before).
pub fn clock_times(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(|c| c.is_ascii_digit());
    let mut out = Vec::new();
    for i in 1..b.len() {
        if b[i] != b':' || !digit(i + 1) || !digit(i + 2) || digit(i + 3) {
            continue;
        }
        let before = (1..=3).take_while(|k| i >= *k && digit(i - k)).count();
        if before == 1 || before == 2 {
            out.push(text[i - before..i + 3].to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_clock_times() {
        assert_eq!(clock_times("Sun enters the gap at 19:58, arrive 7:15 pm"), vec!["19:58", "7:15"]);
        assert!(clock_times("ratio 16:9 and 123:45 and 3:5").is_empty());
        assert!(clock_times("no times here").is_empty());
    }

    #[test]
    fn only_tool_times_pass() {
        let allowed = vec!["19:58".to_string(), "19:15".to_string()];
        assert!(times_ok("The sun drops into the gap at 19:58; arrive by 7:15.", &allowed));
        assert!(!times_ok("The sun sets at 20:07.", &allowed));
        assert!(times_ok("No numbers at all.", &allowed));
    }
}
