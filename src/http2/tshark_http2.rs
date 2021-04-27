use serde::de;
use serde::Deserialize;
use serde::Deserializer;
use serde_json::Value;

#[derive(Debug)]
pub struct TSharkHttp2Message {
    pub headers: Vec<(String, String)>,
    pub data: Option<Vec<u8>>,
    pub stream_id: u32,
    pub is_end_stream: bool,
}

#[derive(Debug)]
pub struct TSharkHttp2 {
    pub messages: Vec<TSharkHttp2Message>,
}

impl<'de> Deserialize<'de> for TSharkHttp2 {
    fn deserialize<D>(deserializer: D) -> Result<TSharkHttp2, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Value = de::Deserialize::deserialize(deserializer)?;
        let messages = map_ar_or_obj(&s, parse_http2_item)
            .into_iter()
            .flatten()
            .filter(|msg| !msg.headers.is_empty() || matches!(&msg.data, Some(v) if !v.is_empty()))
            .collect();
        Ok(TSharkHttp2 { messages })
        // Err(de::Error::custom("invalid http contents"))
    }
}

fn parse_http2_item(obj: &serde_json::Map<String, Value>) -> Vec<TSharkHttp2Message> {
    let stream = &obj.get("http2.stream");
    stream
        .map(|s| map_ar_or_obj(s, parse_message))
        .unwrap_or_else(Vec::new)
}

pub fn map_ar_or_obj<T>(
    val: &Value,
    mapper: impl Fn(&serde_json::Map<String, Value>) -> T,
) -> Vec<T> {
    match val {
        Value::Object(o) => vec![mapper(&o)],
        Value::Array(vals) => vals
            .iter()
            .filter_map(|v| v.as_object())
            .map(|o| mapper(o))
            .collect(),
        _ => vec![],
    }
}

fn parse_message(obj: &serde_json::Map<String, Value>) -> TSharkHttp2Message {
    let headers = obj
        .get("http2.header")
        .and_then(|h| h.as_array())
        .map(|ar| ar.into_iter().filter_map(|v| parse_header(v)).collect())
        .unwrap_or(vec![]);
    let data = obj
        .get("http2.data.data")
        .and_then(|s| s.as_str())
        .and_then(|s| hex::decode(s.replace(':', "")).ok());
    let stream_id = obj
        .get("http2.streamid")
        .and_then(|sid| sid.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let is_end_stream = obj
        .get("http2.flags_tree")
        .and_then(|t| t.as_object())
        .and_then(|t| t.get("http2.flags.end_stream"))
        .and_then(|s| s.as_str())
        .and_then(|s| s.parse::<u32>().ok())
        .map(|v| v != 0)
        .unwrap_or(false);
    TSharkHttp2Message {
        headers,
        data,
        stream_id,
        is_end_stream,
    }
}

fn parse_header(header: &Value) -> Option<(String, String)> {
    let obj = header.as_object()?;
    let key = obj.get("http2.header.name")?.as_str()?;
    let value = obj.get("http2.header.value")?.as_str()?;
    Some((key.to_string(), value.to_string()))
}
