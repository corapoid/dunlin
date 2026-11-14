use super::*;
use crate::http;
fn cap(value: &mut String) -> bool {
    if value.len() <= http::MAX_RESPONSE_BODY {
        return false;
    }
    let mut end = http::MAX_RESPONSE_BODY;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    true
}
pub(crate) fn redact_output(
    mut output: RecipeOutput,
    value: &str,
    basic_username: Option<&str>,
) -> Result<RecipeOutput, String> {
    let stdout = Zeroizing::new(std::mem::take(&mut output.stdout));
    let stderr = Zeroizing::new(std::mem::take(&mut output.stderr));
    if stdout.len() > http::MAX_RESPONSE_BODY || stderr.len() > http::MAX_RESPONSE_BODY {
        return Err("Recipe output exceeds its limit".into());
    }
    let basic = basic_username.map(|username| Zeroizing::new(format!("{username}:{value}")));
    let basic = basic.as_ref().map(|value| value.as_str());
    output.stdout = http::redact(&stdout, value, basic);
    output.stderr = http::redact(&stderr, value, basic);
    output.truncated |= cap(&mut output.stdout) | cap(&mut output.stderr);
    Ok(output)
}
