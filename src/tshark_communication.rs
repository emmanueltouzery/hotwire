use chrono::NaiveDateTime;
use serde::de;
use serde::Deserialize;
use serde::Deserializer;
use serde_aux::prelude::*;
use serde_json::Value;

#[derive(Deserialize)]
pub struct TSharkCommunication {
    #[serde(rename = "_source")]
    pub source: TSharkSource,
}

#[derive(Deserialize)]
pub struct TSharkSource {
    pub layers: TSharkLayers,
}

#[derive(Deserialize, Debug)]
pub struct TSharkLayers {
    pub frame: TSharkFrameLayer,
    pub ip: Option<TSharkIpLayer>,
    pub ipv6: Option<TSharkIpV6Layer>,
    pub tcp: Option<TSharkTcpLayer>,
    pub http: Option<TSharkHttp>,
    pub pgsql: Option<Value>, // TODO no more value
    pub tls: Option<Value>,   // TODO no more value
}

impl TSharkLayers {
    pub fn ip_src(&self) -> String {
        self.ip
            .as_ref()
            .map(|i| &i.ip_src)
            .unwrap_or_else(|| self.ipv6.as_ref().map(|i| &i.ip_src).unwrap())
            .clone()
    }

    pub fn ip_dst(&self) -> String {
        self.ip
            .as_ref()
            .map(|i| &i.ip_dst)
            .unwrap_or_else(|| self.ipv6.as_ref().map(|i| &i.ip_dst).unwrap())
            .clone()
    }
}

#[derive(Deserialize, Debug)]
pub struct TSharkFrameLayer {
    #[serde(rename = "frame.time", deserialize_with = "parse_frame_time")]
    pub frame_time: NaiveDateTime,
    // #[serde(rename = "frame.time_relative")]
    // pub time_relative: String,
}

#[derive(Deserialize, Debug)]
pub struct TSharkIpLayer {
    #[serde(rename = "ip.src")]
    pub ip_src: String,
    #[serde(rename = "ip.dst")]
    pub ip_dst: String,
}

#[derive(Deserialize, Debug)]
pub struct TSharkIpV6Layer {
    #[serde(rename = "ipv6.src")]
    pub ip_src: String,
    #[serde(rename = "ipv6.dst")]
    pub ip_dst: String,
}

#[derive(Deserialize, Debug)]
pub struct TSharkTcpLayer {
    #[serde(
        rename = "tcp.seq",
        deserialize_with = "deserialize_number_from_string"
    )]
    pub seq_number: u32,
    #[serde(
        rename = "tcp.stream",
        deserialize_with = "deserialize_number_from_string"
    )]
    pub stream: u32,
    #[serde(
        rename = "tcp.srcport",
        deserialize_with = "deserialize_number_from_string"
    )]
    pub port_src: u32,
    #[serde(
        rename = "tcp.dstport",
        deserialize_with = "deserialize_number_from_string"
    )]
    pub port_dst: u32,
}

fn parse_frame_time<'de, D>(deserializer: D) -> Result<NaiveDateTime, D::Error>
where
    D: Deserializer<'de>,
{
    // must use NaiveDateTime because chrono can't read string timezone names.
    // https://docs.rs/chrono/0.4.19/chrono/format/strftime/index.html#specifiers
    // > %Z: Offset will not be populated from the parsed data, nor will it be validated.
    // > Timezone is completely ignored. Similar to the glibc strptime treatment of this format code.
    // > It is not possible to reliably convert from an abbreviation to an offset, for example CDT
    // > can mean either Central Daylight Time (North America) or China Daylight Time.
    let s: String = de::Deserialize::deserialize(deserializer)?;
    NaiveDateTime::parse_from_str(&s, "%b %e, %Y %T.%f %Z").map_err(de::Error::custom)
}

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
