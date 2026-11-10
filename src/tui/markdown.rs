use super::view::Theme;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};

/// Deliberately conservative: render text, never execute escapes or open links.
pub fn render(content: &str, theme: Theme) -> Text<'static> {
    let safe = crate::cli::safe_terminal_content(content).replace('\t', "    ");
    let mut code = false;
    let mut lines = Vec::new();
    for raw in safe.lines() {
        let text = raw.trim_end();
        if text.trim_start().starts_with("```") {
            code = !code;
            lines.push(Line::from(Span::styled(
                if code {
                    format!("  {}", text.trim_start().trim_start_matches('`'))
                } else {
                    String::new()
                },
                theme.muted(),
            )));
        } else if code {
            lines.push(Line::from(Span::styled(format!("  {text}"), theme.code())));
        } else if let Some(heading) = text
            .strip_prefix("### ")
            .or_else(|| text.strip_prefix("## "))
            .or_else(|| text.strip_prefix("# "))
        {
            lines.push(Line::from(Span::styled(
                heading.to_owned(),
                theme.strong().add_modifier(Modifier::BOLD),
            )));
        } else if let Some(quote) = text.strip_prefix("> ") {
            let mut spans = vec![Span::styled("│  ", theme.accent())];
            spans.extend(inline(
                quote,
                theme.muted().add_modifier(Modifier::ITALIC),
                theme,
            ));
            lines.push(Line::from(spans));
        } else if matches!(text, "---" | "***" | "___") {
            lines.push(Line::from(Span::styled(
                "────────────────────────",
                theme.rule(),
            )));
        } else {
            let text = if let Some(rest) = text
                .strip_prefix("- [x] ")
                .or_else(|| text.strip_prefix("- [X] "))
            {
                format!("[✓] {rest}")
            } else if let Some(rest) = text.strip_prefix("- [ ] ") {
                format!("[ ] {rest}")
            } else if let Some(rest) = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")) {
                format!("• {rest}")
            } else {
                text.to_owned()
            };
            lines.push(Line::from(inline(&text, theme.text(), theme)));
        }
    }
    Text::from(lines)
}
fn inline(text: &str, base: Style, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = vec![];
    let mut rest = text;
    while !rest.is_empty() {
        let next = ["**", "`"]
            .into_iter()
            .filter_map(|mark| rest.find(mark).map(|at| (at, mark)))
            .min_by_key(|(at, _)| *at);
        let Some((at, mark)) = next else {
            spans.push(Span::styled(rest.to_owned(), base));
            break;
        };
        let after = &rest[at + mark.len()..];
        if let Some(end) = after.find(mark) {
            if at > 0 {
                spans.push(Span::styled(rest[..at].to_owned(), base));
            }
            let style = if mark == "`" {
                theme.code()
            } else {
                base.add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(after[..end].to_owned(), style));
            rest = &after[end + mark.len()..];
        } else {
            spans.push(Span::styled(rest.to_owned(), base));
            break;
        }
    }
    spans
}
