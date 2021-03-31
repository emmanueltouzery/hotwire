use serde::de;
use serde::Deserialize;
use serde::Deserializer;
use serde_json::Value;

#[derive(Debug, Copy, Clone)]
pub enum HttpType {
    Request,
    Response,
}

#[derive(Debug)]
pub struct TSharkHttp {
    pub http_type: HttpType,
    pub http_host: Option<String>,
    pub first_line: String,
    pub other_lines: String,
    pub body: Option<String>,
    pub content_type: Option<String>,
}

fn extract_first_line(http_map: &serde_json::Map<String, Value>, key_name: &str) -> String {
    http_map
        .iter()
        .find(|(_k, v)| {
            matches!(v, serde_json::Value::Object(fields) if fields.contains_key(key_name))
        })
        .map(|(k, _v)| k.as_str())
        .unwrap_or("")
        .trim_end_matches("\\r\\n")
        .to_string()
}

impl<'de> Deserialize<'de> for TSharkHttp {
    fn deserialize<D>(deserializer: D) -> Result<TSharkHttp, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Value = de::Deserialize::deserialize(deserializer)?;
        let http_map = s.as_object().unwrap();
        let body = http_map
            .get("http.file_data")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string());
        if let Some(req_line) = http_map.get("http.request.line") {
            let other_lines_vec: Vec<_> = req_line
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            return Ok(TSharkHttp {
                http_type: HttpType::Request,
                first_line: extract_first_line(http_map, "http.request.method"),
                other_lines: other_lines_vec.join(""),
                http_host: http_map
                    .get("http.host")
                    .and_then(|c| c.as_str())
                    .map(|c| c.to_string()),
                content_type: http_map
                    .get("http.content_type")
                    .and_then(|c| c.as_str())
                    .map(|c| c.to_string()),
                body,
            });
        }
        if let Some(resp_line) = http_map.get("http.response.line") {
            let other_lines_vec: Vec<_> = resp_line
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            return Ok(TSharkHttp {
                http_type: HttpType::Response,
                first_line: extract_first_line(http_map, "http.response.code"),
                other_lines: other_lines_vec.join(""),
                http_host: http_map
                    .get("http.host")
                    .and_then(|c| c.as_str())
                    .map(|c| c.to_string()),
                content_type: http_map
                    .get("http.content_type")
                    .and_then(|c| c.as_str())
                    .map(|c| c.to_string()),
                body,
            });
        }
        Err(de::Error::custom("invalid http contents"))
    }
}
