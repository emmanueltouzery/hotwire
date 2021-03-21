use serde::de;
use serde::Deserialize;
use serde::Deserializer;

#[derive(Deserialize)]
pub struct TSharkCommunicationRaw {
    #[serde(rename = "_source")]
    pub source: TSharkSourceRaw,
}

#[derive(Deserialize)]
pub struct TSharkSourceRaw {
    pub layers: TSharkLayersRaw,
}

#[derive(Deserialize)]
pub struct TSharkLayersRaw {
    pub http: Option<TSharkLayerHttpRaw>,
}

#[derive(Deserialize)]
pub struct TSharkLayerHttpRaw {
    #[serde(rename = "http.file_data_raw", deserialize_with = "parse_bytes")]
    pub file_data: Vec<u8>,
}

fn parse_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = de::Deserialize::deserialize(deserializer)?;
    Ok(hex::decode(s)?)
}
