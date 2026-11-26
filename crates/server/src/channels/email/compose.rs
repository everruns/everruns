//! Templates are immutable source plus escaped scalar substitutions. No code,
//! includes, network, arbitrary expressions, or provider-hosted rendering.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmailTemplate {
    pub name: String,
    pub subject: String,
    pub html: Option<String>,
    pub text: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ComposedEmail {
    pub subject: String,
    pub text: String,
    pub html: String,
}

pub fn render(
    template: &EmailTemplate,
    variables: &BTreeMap<String, String>,
) -> anyhow::Result<ComposedEmail> {
    anyhow::ensure!(
        variables.len() <= 64 && variables.values().map(String::len).sum::<usize>() <= 65_536,
        "template variables exceed limits"
    );
    let subject = substitute(&template.subject, variables, false)?;
    anyhow::ensure!(
        subject.len() <= 998 && !subject.chars().any(char::is_control),
        "invalid subject"
    );
    let text = template
        .text
        .as_ref()
        .map(|s| substitute(s, variables, false))
        .transpose()?;
    let html = template
        .html
        .as_ref()
        .map(|s| substitute(s, variables, true))
        .transpose()?;
    anyhow::ensure!(
        text.is_some() || html.is_some(),
        "template needs HTML or text"
    );
    let html = match html {
        Some(html) => ammonia::Builder::default()
            .url_relative(ammonia::UrlRelative::Deny)
            .clean(&html)
            .to_string(),
        None => format!("<pre>{}</pre>", escape(text.as_deref().unwrap_or_default())),
    };
    let text = match text {
        Some(text) => text,
        None => markkit::Markkit::builder()
            .limits(
                markkit::Limits::default()
                    .max_input_size(262_144)
                    .max_output_size(262_144),
            )
            .preserve_images(false)
            .build()
            .extract_html_sync(&html)?,
    };
    anyhow::ensure!(
        !text.trim().is_empty() && text.len() + html.len() <= 524_288,
        "empty or oversized email"
    );
    Ok(ComposedEmail {
        subject,
        text,
        html,
    })
}
fn substitute(source: &str, vars: &BTreeMap<String, String>, html: bool) -> anyhow::Result<String> {
    anyhow::ensure!(source.len() <= 262_144, "template too large");
    let mut rest = source;
    let mut output = String::new();
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("unclosed variable"))?;
        let name = tail[..end].trim();
        anyhow::ensure!(
            !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "only named scalar variables are supported"
        );
        let value = vars
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("missing template variable: {name}"))?;
        if html {
            output.push_str(&escape(value));
        } else {
            output.push_str(value);
        }
        anyhow::ensure!(output.len() <= 262_144, "rendered template too large");
        rest = &tail[end + 2..];
    }
    output.push_str(rest);
    anyhow::ensure!(output.len() <= 262_144, "rendered template too large");
    Ok(output)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_is_escaped_and_alternatives_automatic() {
        let t = EmailTemplate {
            name: "Reply".into(),
            subject: "Hello {{name}}".into(),
            html: Some("<p>{{name}}</p><script>bad()</script>".into()),
            text: None,
        };
        let v = render(
            &t,
            &BTreeMap::from([("name".into(), "<img src=x onerror=bad()>".into())]),
        )
        .unwrap();
        assert!(!v.html.contains("<img"));
        assert!(!v.html.contains("<script"));
        assert!(!v.text.is_empty());
        assert!(render(&t, &BTreeMap::new()).is_err());
        assert!(
            render(
                &t,
                &BTreeMap::from([("name".into(), "x\r\nBcc: attacker@example.com".into())])
            )
            .is_err()
        );
    }
}
