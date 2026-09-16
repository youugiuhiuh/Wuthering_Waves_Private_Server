use crate::common::Markup;

/// 把 inline keyboard markup 渲染为文本命令列表，供不支持 inline keyboard 的平台使用。
pub fn render_markup_buttons(base: String, markup: &Markup) -> String {
    let mut body = base;
    let mut lines: Vec<String> = Vec::new();
    let mut idx = 1;
    for row in &markup.buttons {
        for btn in row {
            lines.push(format!("{}. {} — send: `{}`", idx, btn.text, btn.data));
            idx += 1;
        }
    }
    if !lines.is_empty() {
        body.push_str(&rust_i18n::t!("matrix.markup_header"));
        body.push_str(&lines.join("\n"));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::InlineButton;

    #[test]
    fn renders_buttons_as_numbered_commands() {
        let markup = Markup {
            buttons: vec![vec![InlineButton {
                text: "Help".into(),
                data: "/help".into(),
            }]],
        };
        let out = render_markup_buttons("Hi".into(), &markup);
        assert!(out.starts_with("Hi"));
        assert!(out.contains("1. Help — send: `/help`"));
    }

    #[test]
    fn leaves_plain_text_untouched() {
        let out = render_markup_buttons("plain".into(), &Markup { buttons: vec![] });
        assert_eq!(out, "plain");
    }
}
