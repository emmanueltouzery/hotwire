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

// the format is weird. the image bytes are the first element of the array,
// not sure what the others are, ignoring them for now.
//
//    "http.file_data_raw": [
//     "89504e470d0a1a0a...
//     388,
//     7934,
//     0,
//     26
//   ]
fn parse_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Vec<serde_json::Value> = de::Deserialize::deserialize(deserializer)?;
    hex::decode(s.first().unwrap().as_str().unwrap()).map_err(de::Error::custom)
}
