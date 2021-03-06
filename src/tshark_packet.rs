use serde::de;
use serde_derive::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct TSharkPacket {
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
}

#[derive(Deserialize)]
pub struct TSharkIpLayer {
    #[serde(rename = "ip.src")]
    pub ip_src: String,
}

#[derive(Deserialize)]
pub struct TSharkTcpLayer {
    #[serde(rename = "tcp.stream")]
    pub stream: String,
}
