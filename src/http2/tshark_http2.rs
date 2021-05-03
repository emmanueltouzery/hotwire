use crate::serde_multival::MultiVal;
use serde::de;
use serde::Deserialize;
use serde::Deserializer;
use serde_json::Value;
use std::fmt::Debug;

// if i have data over 2 http2 packets, tshark will often give me
// the first part of the data in the first packet, then
// the RECOMPOSED data in the second packet, repeating data from
// the first packet.
// => when combining packets, the recomposed data should overwrite
// previously collected data
pub enum Http2Data {
    BasicData(Vec<u8>),
    RecomposedData(Vec<u8>),
}

impl Debug for Http2Data {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match &self {
            Http2Data::BasicData(d) => fmt.write_str(&format!("BasicData(length: {})", d.len())),
            Http2Data::RecomposedData(d) => {
                fmt.write_str(&format!("RecomposedData(length: {})", d.len()))
            }
        }
    }
}

impl Http2Data {
    fn is_empty(&self) -> bool {
        match &self {
            Http2Data::BasicData(v) => v.is_empty(),
            Http2Data::RecomposedData(v) => v.is_empty(),
        }
    }
}

#[derive(Debug)]
pub struct TSharkHttp2Message {
    pub headers: Vec<(String, String)>,
    pub data: Option<Http2Data>,
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
        let s: MultiVal = de::Deserialize::deserialize(deserializer)?;
        dbg!(&s);
        // let messages = map_ar_or_obj(&s, parse_http2_item)
        //     .into_iter()
        //     .flatten()
        //     .filter(|msg| !msg.headers.is_empty() || matches!(&msg.data, Some(v) if !v.is_empty()))
        //     .collect();
        Ok(TSharkHttp2 { messages: vec![] })
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
    let data = read_data(&obj);
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

fn read_data(obj: &serde_json::Map<String, serde_json::Value>) -> Option<Http2Data> {
    obj.get("http2.data.data")
        .and_then(|s| s.as_str())
        .and_then(|s| hex::decode(s.replace(':', "")).ok())
        .map(Http2Data::BasicData)
        .or_else(|| {
            // we didn't find directly a field "http2.data.data", but sometimes tshark will decode for us
            // and create "Content-encoded ....": { "http2.data.data": "...", ... }
            // => search for a field that would CONTAIN A FIELD named http2.data.data
            obj.iter()
                .find(|(_k, v)| {
                    v.as_object()
                        .filter(|o| o.contains_key("http2.data.data"))
                        .is_some()
                })
                .and_then(|(_k, v)| v.as_object())
                .and_then(|o| o.get("http2.data.data"))
                .and_then(|s| s.as_str())
                .and_then(|s| hex::decode(s.replace(':', "")).ok())
                .map(Http2Data::RecomposedData)
        })
}

fn parse_header(header: &Value) -> Option<(String, String)> {
    let obj = header.as_object()?;
    let key = obj.get("http2.header.name")?.as_str()?;
    let value = obj.get("http2.header.value")?.as_str()?;
    Some((key.to_string(), value.to_string()))
}
