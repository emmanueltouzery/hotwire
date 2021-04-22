use crate::http::tshark_http;
use crate::http2::tshark_http2;
use crate::pgsql::tshark_pgsql;
use chrono::NaiveDateTime;
use serde::de;
use serde::Deserialize;
use serde::Deserializer;
use serde_aux::prelude::*;

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
    pub http: Option<tshark_http::TSharkHttp>,
    pub http2: Option<tshark_http2::TSharkHttp2>,
    pub pgsql: Option<tshark_pgsql::TSharkPgsql>,
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
        rename = "tcp.seq_raw",
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
