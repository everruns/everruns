//! Fixed activity wording only. Argument values and authored labels never enter this lookup.
use super::BackendLocale;
#[path = "activity/de.rs"]
mod de;
#[path = "activity/es.rs"]
mod es;
#[path = "activity/fr.rs"]
mod fr;
#[path = "activity/zh_cn.rs"]
mod zh_cn;

pub fn runtime_text<'a>(locale: Option<&str>, source: &'a str) -> &'a str {
    match super::resolve_backend_locale(locale) {
        BackendLocale::Fr => fr::translate(source),
        BackendLocale::De => de::translate(source),
        BackendLocale::Es => es::translate(source),
        BackendLocale::Zh => zh_cn::translate(source),
        _ => source,
    }
}
