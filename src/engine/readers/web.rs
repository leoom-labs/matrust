//! webread, websave and weboptions: HTTP and HTTPS GET requests through the
//! system `curl` program (part of Windows 10 and later, macOS and most Linux
//! systems), so requests use the system's certificate store and proxy
//! variables and the download adds no TLS stack. Not in the browser build.
//! Behaviour follows the published MathWorks function pages (behaviour only);
//! differences are listed on the help pages.
// The request helpers are unused where webread and websave only report an error.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
use super::{rdx_number_text, rdx_pairs, rdx_text};
use crate::{
    Array, CellArray, CharacterRow, Matrix, Result, StructArray, engine::Engine, engine::Value,
};
use std::sync::Arc;

/// Default and largest request timeout in seconds.
const DEFAULT_TIMEOUT: f64 = 5.;
const MAX_TIMEOUT: f64 = 86_400.;
/// Largest response webread keeps in memory, and largest file websave writes.
#[cfg(not(target_arch = "wasm32"))]
const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_SAVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const OPTION_FIELDS: [&str; 5] = [
    "CharacterEncoding",
    "UserAgent",
    "Timeout",
    "ContentType",
    "HeaderFields",
];
const CONTENT_TYPES: [&str; 5] = ["auto", "text", "json", "binary", "raw"];

/// Request settings from weboptions.
#[derive(Clone, Debug)]
struct WebOptions {
    encoding: String,
    user_agent: String,
    timeout: f64,
    content_type: String,
    headers: Vec<(String, String)>,
}

impl Default for WebOptions {
    fn default() -> Self {
        Self {
            encoding: "auto".into(),
            user_agent: format!("MatRust/{}", env!("CARGO_PKG_VERSION")),
            timeout: DEFAULT_TIMEOUT,
            content_type: "auto".into(),
            headers: Vec::new(),
        }
    }
}

fn web_header_text(function: &str, text: &str) -> Result<String> {
    if text.contains(['\r', '\n', '\0']) {
        return Err(format!(
            "{function}: header names and values cannot contain line breaks"
        ));
    }
    Ok(text.to_owned())
}

/// HeaderFields: an n-by-2 cell array or string array of names and values.
fn web_headers(function: &str, value: &Value) -> Result<Vec<(String, String)>> {
    let bad = || format!("{function}: HeaderFields must be an n-by-2 cell array of text");
    let item_text = |a: &Array| match a {
        Array::Character(ch) if ch.rows() <= 1 => Some(String::from_utf16_lossy(ch.units())),
        Array::Str(s) if s.len() == 1 => s.data()[0].as_deref().map(str::to_owned),
        _ => None,
    };
    let mut texts = Vec::new();
    let (rows, cols) = match value {
        Value::Container(Array::Cell(c)) => {
            for i in 0..c.len() {
                texts.push(item_text(c.get_linear(i)?).ok_or_else(bad)?);
            }
            (c.rows(), c.cols())
        }
        Value::Container(Array::Str(s)) => {
            for item in s.data() {
                texts.push(item.as_deref().ok_or_else(bad)?.to_owned());
            }
            let dims = Array::Str(s.clone()).dims().into_owned();
            (dims[0], dims.get(1).copied().unwrap_or(1))
        }
        Value::Matrix(m) if m.is_empty() => return Ok(Vec::new()),
        _ => return Err(bad()),
    };
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    if cols != 2 {
        return Err(bad());
    }
    let mut headers = Vec::with_capacity(rows);
    for r in 0..rows {
        let name = web_header_text(function, &texts[r])?;
        if name.is_empty() || name.contains(':') {
            return Err(format!("{function}: '{name}' is not a valid header name"));
        }
        headers.push((name, web_header_text(function, &texts[r + rows])?));
    }
    Ok(headers)
}

fn web_set_option(
    function: &str,
    options: &mut WebOptions,
    name: &str,
    value: &Value,
) -> Result<()> {
    let text =
        |what: &str| rdx_text(value).ok_or_else(|| format!("{function}: {what} must be text"));
    match name.to_ascii_lowercase().as_str() {
        "characterencoding" => options.encoding = text("CharacterEncoding")?,
        "useragent" => options.user_agent = web_header_text(function, &text("UserAgent")?)?,
        "timeout" => {
            let t = match value {
                Value::Matrix(m) if m.len() == 1 => m.data()[0],
                Value::Container(Array::Typed(t)) if t.len() == 1 => t.real(0),
                _ => f64::NAN,
            };
            if t.is_nan() || t <= 0. {
                return Err(format!(
                    "{function}: Timeout must be a positive number of seconds"
                ));
            }
            options.timeout = t.min(MAX_TIMEOUT);
        }
        "contenttype" => {
            let t = text("ContentType")?.to_ascii_lowercase();
            if !CONTENT_TYPES.contains(&t.as_str()) {
                return Err(format!(
                    "{function}: ContentType '{t}' is not supported; use 'auto', 'text', 'json', 'binary' or 'raw'"
                ));
            }
            options.content_type = t;
        }
        "headerfields" => options.headers = web_headers(function, value)?,
        "username" | "password" | "keyname" | "keyvalue" | "certificatefilename" => {
            return Err(format!(
                "{function}: the option '{name}' is not supported (no authentication options yet)"
            ));
        }
        "requestmethod" => match rdx_text(value).map(|t| t.to_ascii_lowercase()).as_deref() {
            Some("auto" | "get") => {}
            _ => return Err(format!("{function}: only RequestMethod 'get' is supported")),
        },
        "mediatype" | "arrayformat" | "contentreader" => {}
        _ => {
            return Err(format!(
                "{function}: unknown option '{name}'; supported are {}",
                OPTION_FIELDS.join(", ")
            ));
        }
    }
    Ok(())
}

/// A struct made by weboptions (or by hand with those field names).
fn web_options_struct(value: &Value) -> Option<&StructArray> {
    match value {
        Value::Container(Array::Struct(s))
            if s.len() == 1
                && s.field_names()
                    .iter()
                    .any(|f| OPTION_FIELDS.contains(&f.as_str())) =>
        {
            Some(s)
        }
        _ => None,
    }
}

fn web_from_struct(function: &str, s: &StructArray) -> Result<WebOptions> {
    let mut options = WebOptions::default();
    for (i, name) in s.field_names().iter().enumerate() {
        let value = Value::from((**s.get(i, 0)?).clone());
        web_set_option(function, &mut options, name, &value)?;
    }
    Ok(options)
}

/// Percent-encoding of a query name or value (RFC 3986 unreserved kept).
fn web_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Text of a query value: text as is, numbers as comma-separated values.
fn web_query_value(function: &str, value: &Value) -> Result<String> {
    if let Some(t) = rdx_text(value) {
        return Ok(t);
    }
    let numbers: Vec<f64> = match value {
        Value::Matrix(m) => m.data().to_vec(),
        Value::Logical(m) => m.data().iter().map(|&b| f64::from(b)).collect(),
        Value::Container(Array::Typed(t)) => t.to_f64_vec()?,
        _ => {
            return Err(format!(
                "{function}: query values must be text or real numbers"
            ));
        }
    };
    Ok(numbers
        .iter()
        .map(|&x| {
            if x.is_nan() {
                "NaN".to_owned()
            } else if x.is_infinite() {
                if x > 0. { "Inf".into() } else { "-Inf".into() }
            } else {
                rdx_number_text(x)
            }
        })
        .collect::<Vec<_>>()
        .join(","))
}

/// The URL with query pairs appended, and the options (an options struct is
/// the last argument when given).
fn web_request(function: &str, args: &[Value]) -> Result<(String, WebOptions)> {
    let Some((url, mut rest)) = args.split_first() else {
        return Err("Not enough input arguments.".into());
    };
    let mut url = rdx_text(url).ok_or_else(|| format!("{function}: the URL must be text"))?;
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) || url.len() < 9 {
        return Err(format!(
            "{function}: the URL must start with http:// or https://"
        ));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!(
            "{function}: the URL cannot contain spaces or control characters; pass query values as name/value pairs"
        ));
    }
    let mut options = WebOptions::default();
    if let Some((last, head)) = rest.split_last()
        && let Some(s) = web_options_struct(last)
    {
        options = web_from_struct(function, s)?;
        rest = head;
    }
    if !rest.len().is_multiple_of(2) {
        return Err(format!(
            "{function}: query parameters must be name/value pairs"
        ));
    }
    let mut query = Vec::with_capacity(rest.len() / 2);
    for pair in rest.as_chunks::<2>().0 {
        let name =
            rdx_text(&pair[0]).ok_or_else(|| format!("{function}: query names must be text"))?;
        let value = web_query_value(function, &pair[1])?;
        query.push(format!("{}={}", web_encode(&name), web_encode(&value)));
    }
    if !query.is_empty() {
        let (base, fragment) = match url.split_once('#') {
            Some((b, f)) => (b.to_owned(), Some(f.to_owned())),
            None => (url.clone(), None),
        };
        let joiner = if !base.contains('?') {
            "?"
        } else if base.ends_with(['?', '&']) {
            ""
        } else {
            "&"
        };
        url = format!("{base}{joiner}{}", query.join("&"));
        if let Some(f) = fragment {
            url = format!("{url}#{f}");
        }
    }
    Ok((url, options))
}

/// Decoded text of a response body.
fn web_text(body: &[u8], charset: Option<&str>, options: &WebOptions) -> String {
    let encoding = if options.encoding.eq_ignore_ascii_case("auto") {
        charset.unwrap_or("utf-8").to_ascii_lowercase()
    } else {
        options.encoding.to_ascii_lowercase()
    };
    let body = body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body);
    match encoding.as_str() {
        "iso-8859-1" | "latin1" | "latin-1" | "us-ascii" | "windows-1252" => {
            body.iter().map(|&b| b as char).collect()
        }
        _ => String::from_utf8_lossy(body).into_owned(),
    }
}

/// How webread returns a body of media type `media` ('auto' content type).
fn web_kind(media: &str) -> &'static str {
    let media = media.trim();
    if media == "application/json" || media == "text/json" || media.ends_with("+json") {
        "json"
    } else if media.starts_with("text/")
        || media.is_empty()
        || media.ends_with("+xml")
        || matches!(
            media,
            "application/xml" | "application/javascript" | "application/x-www-form-urlencoded"
        )
    {
        "text"
    } else {
        "binary"
    }
}

impl Engine {
    pub(super) fn web_options(&self, args: &[Value]) -> Result<Value> {
        let mut options = WebOptions::default();
        for (name, value) in rdx_pairs("weboptions", args)? {
            web_set_option("weboptions", &mut options, &name, value)?;
        }
        let text = |t: &str| -> Result<Arc<Array>> {
            Ok(Arc::new(CharacterRow::from_text(t, &self.cancel)?.into()))
        };
        let headers = if options.headers.is_empty() {
            Arc::new(Matrix::filled(0, 0, 0.)?.into())
        } else {
            let n = options.headers.len();
            let mut slots = Vec::with_capacity(2 * n);
            for (name, _) in &options.headers {
                slots.push(text(name)?);
            }
            for (_, value) in &options.headers {
                slots.push(text(value)?);
            }
            Arc::new(Array::Cell(CellArray::from_shared(
                n,
                2,
                slots,
                &self.cancel,
            )?))
        };
        let slots = vec![
            text(&options.encoding)?,
            text(&options.user_agent)?,
            Arc::new(Matrix::scalar(options.timeout).into()),
            text(&options.content_type)?,
            headers,
        ];
        let names = OPTION_FIELDS.iter().map(|s| (*s).to_owned()).collect();
        Ok(Array::Struct(StructArray::from_slots(1, 1, names, slots, &self.cancel)?).into())
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn web_read(&mut self, _args: &[Value]) -> Result<Value> {
        Err("webread is not available in the browser build; use the desktop or command-line MatRust".into())
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn web_save(&mut self, _args: &[Value], _nargout: usize) -> Result<Vec<Value>> {
        Err("websave is not available in the browser build; use the desktop or command-line MatRust".into())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn web_read(&mut self, args: &[Value]) -> Result<Value> {
        let (url, options) = web_request("webread", args)?;
        let dir = tempfile::tempdir().map_err(|e| format!("webread: {e}"))?;
        let body_path = dir.path().join("body");
        let response = self.web_get("webread", &url, &options, &body_path, MAX_READ_BYTES)?;
        let body = std::fs::read(&body_path).map_err(|e| format!("webread: {e}"))?;
        let kind = match options.content_type.as_str() {
            "auto" => web_kind(&response.media),
            "raw" => match web_kind(&response.media) {
                "json" => "text",
                k => k,
            },
            "text" => "text",
            "json" => "json",
            _ => "binary",
        };
        match kind {
            "binary" => {
                let n = body.len();
                let data = crate::TypedArray::from_ints(
                    crate::typed_array::NumericClass::Uint8,
                    n,
                    1,
                    body.into_iter().map(i128::from),
                )?;
                Ok(Array::Typed(data).into())
            }
            "json" => {
                let text = web_text(&body, response.charset.as_deref(), &options);
                self.rdx_jsondecode(&[Value::Text(text)])
                    .map_err(|e| format!("webread: the response is not valid JSON: {e}"))
            }
            _ => {
                let text = web_text(&body, response.charset.as_deref(), &options);
                Ok(Value::from(Array::from(CharacterRow::from_text(
                    &text,
                    &self.cancel,
                )?)))
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn web_save(&mut self, args: &[Value], _nargout: usize) -> Result<Vec<Value>> {
        let Some((file, rest)) = args.split_first() else {
            return Err("Not enough input arguments.".into());
        };
        let filename = rdx_text(file).ok_or("websave: the file name must be text")?;
        let (url, options) = web_request("websave", rest)?;
        let path = self.file_path(&filename, "")?;
        if path.is_dir() {
            return Err(format!("websave: {filename} is a folder"));
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        // Downloaded next to the target, then renamed, so a failed request
        // leaves an existing file untouched.
        let temp = tempfile::Builder::new()
            .prefix(".websave-")
            .tempfile_in(parent)
            .map_err(|e| format!("websave: unable to write in {}: {e}", parent.display()))?
            .into_temp_path();
        self.web_get("websave", &url, &options, &temp, MAX_SAVE_BYTES)?;
        temp.persist(&path)
            .map_err(|e| format!("websave: unable to write {filename}: {}", e.error))?;
        let full = std::path::absolute(&path).unwrap_or(path);
        Ok(vec![Value::from(Array::from(CharacterRow::from_text(
            &full.to_string_lossy(),
            &self.cancel,
        )?))])
    }

    /// GET `url` into `body_path` with curl. Fails on an HTTP status of 400
    /// or more, on a body over `max_bytes` and on cancellation.
    #[cfg(not(target_arch = "wasm32"))]
    fn web_get(
        &self,
        function: &str,
        url: &str,
        options: &WebOptions,
        body_path: &std::path::Path,
        max_bytes: u64,
    ) -> Result<WebResponse> {
        use std::{
            io::Read,
            process::{Command, Stdio},
            sync::atomic::Ordering,
            time::Duration,
        };
        let mut command = Command::new("curl");
        // --disable (first) ignores .curlrc; --globoff keeps [] and {} literal.
        command
            .arg("--disable")
            .args(["--silent", "--show-error", "--globoff", "--location"])
            .args(["--max-redirs", "10"])
            .args(["--proto", "=http,https", "--proto-redir", "=http,https"])
            .args(["--max-time", &format!("{}", options.timeout)])
            .args(["--max-filesize", &max_bytes.to_string()])
            .args(["--user-agent", &options.user_agent])
            .args(["--write-out", "%{http_code}\\n%{content_type}"])
            .arg("--output")
            .arg(body_path);
        for (name, value) in &options.headers {
            command.args(["--header", &format!("{name}: {value}")]);
        }
        command
            .args(["--url", url])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console flashes up from the desktop app.
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "{function} needs the curl program, which was not found; install curl (it ships with Windows 10 and later and macOS)"
                )
            } else {
                format!("{function}: unable to start curl: {e}")
            }
        })?;
        // Both pipes are drained while curl runs (a long --write-out or error
        // text must not block it); only the first 64 KiB of each are kept.
        fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
            std::thread::spawn(move || {
                let mut kept = Vec::new();
                if let Some(mut pipe) = pipe {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = pipe.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        let room = (64 * 1024usize).saturating_sub(kept.len());
                        kept.extend_from_slice(&buf[..n.min(room)]);
                    }
                }
                String::from_utf8_lossy(&kept).into_owned()
            })
        }
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        // curl enforces --max-time; this deadline covers a curl that does not exit.
        let deadline = std::time::Instant::now() + Duration::from_secs_f64(options.timeout + 30.);
        let stop = |child: &mut std::process::Child| {
            let _ = child.kill();
            let _ = child.wait();
        };
        let status = loop {
            if self.cancel.load(Ordering::Relaxed) {
                stop(&mut child);
                return Err("Calculation cancelled".into());
            }
            if let Some(status) = child.try_wait().map_err(|e| format!("{function}: {e}"))? {
                break status;
            }
            if std::time::Instant::now() > deadline {
                stop(&mut child);
                return Err(format!(
                    "{function}: could not read {url}: no complete response within {} s (Timeout)",
                    options.timeout
                ));
            }
            // Bodies sent without a length are bounded here.
            if std::fs::metadata(body_path).is_ok_and(|m| m.len() > max_bytes) {
                stop(&mut child);
                return Err(format!(
                    "{function}: the response exceeds {} MiB",
                    max_bytes >> 20
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let out = stdout.join().unwrap_or_default();
        let err = stderr.join().unwrap_or_default();
        if !status.success() {
            let code = status.code().unwrap_or(-1);
            let reason = err
                .lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| {
                    // "curl: (6) Could not resolve host: x" -> "Could not resolve host: x"
                    let l = l.trim().trim_start_matches("curl: ");
                    match l.strip_prefix('(').and_then(|r| r.split_once(") ")) {
                        Some((n, rest)) if n.bytes().all(|b| b.is_ascii_digit()) => rest.to_owned(),
                        _ => l.to_owned(),
                    }
                })
                .unwrap_or_default();
            let reason = match code {
                28 => format!(
                    "no complete response within {} s (Timeout)",
                    options.timeout
                ),
                63 => format!("the response exceeds {} MiB", max_bytes >> 20),
                _ if !reason.is_empty() => reason,
                _ => format!("curl failed with exit code {code}"),
            };
            return Err(format!("{function}: could not read {url}: {reason}"));
        }
        let (code, content_type) = out.split_once('\n').unwrap_or((out.as_str(), ""));
        let code: u32 = code.trim().parse().unwrap_or(0);
        if !(200..400).contains(&code) {
            return Err(format!(
                "{function}: the server returned status {code} for {url}"
            ));
        }
        if std::fs::metadata(body_path).is_ok_and(|m| m.len() > max_bytes) {
            return Err(format!(
                "{function}: the response exceeds {} MiB",
                max_bytes >> 20
            ));
        }
        // A response without a body leaves no file behind.
        if !body_path.exists() {
            std::fs::write(body_path, b"").map_err(|e| format!("{function}: {e}"))?;
        }
        let content_type = content_type.trim().to_ascii_lowercase();
        let mut parts = content_type.split(';');
        let media = parts.next().unwrap_or("").trim().to_owned();
        let charset = parts.find_map(|p| {
            p.trim()
                .strip_prefix("charset=")
                .map(|c| c.trim_matches('"').to_owned())
        });
        Ok(WebResponse { media, charset })
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct WebResponse {
    media: String,
    charset: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_pairs_are_percent_encoded_and_appended() {
        let args = [
            Value::Text("https://example.org/api?x=1#top".into()),
            Value::Text("q".into()),
            Value::Text("a b&c".into()),
            Value::Text("n".into()),
            Value::Matrix(Matrix::scalar(2.5)),
        ];
        let (url, _) = web_request("webread", &args).unwrap();
        assert_eq!(url, "https://example.org/api?x=1&q=a%20b%26c&n=2.5#top");
    }

    #[test]
    fn only_http_urls_are_accepted() {
        for bad in ["file:///etc/passwd", "ftp://x.org/a", "https://a b.org"] {
            assert!(
                web_request("webread", &[Value::Text(bad.into())]).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn media_types_pick_the_output() {
        assert_eq!(web_kind("application/json"), "json");
        assert_eq!(web_kind("application/vnd.api+json"), "json");
        assert_eq!(web_kind("text/html"), "text");
        assert_eq!(web_kind("image/png"), "binary");
    }
}
