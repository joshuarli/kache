//! Lexical path forms shared by configuration and compiler normalization.

pub fn strip_verbatim_prefix(s: &str) -> std::borrow::Cow<'_, str> {
    std::borrow::Cow::Borrowed(s)
}

pub fn is_filesystem_root_prefix(value: &str) -> bool {
    value == "/"
}
