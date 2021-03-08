use serde_aux::prelude::*;
use serde_derive::Deserialize;
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

#[derive(Deserialize)]
pub struct TSharkLayers {
    pub ip: Option<TSharkIpLayer>,
    pub tcp: Option<TSharkTcpLayer>,
    pub http: Option<Value>,
    pub pgsql: Option<Value>,
}

#[derive(Deserialize)]
pub struct TSharkIpLayer {
    #[serde(rename = "ip.src")]
    pub ip_src: String,
    #[serde(rename = "ip.dst")]
    pub ip_dst: String,
}

#[derive(Deserialize)]
pub struct TSharkTcpLayer {
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
