use serde_json::Value;

// not validated... but at least we check it's a JSON Object.
pub struct TSharkPacket {
    data: serde_json::Map<String, Value>,
}

impl TSharkPacket {
    pub fn new(value: Value) -> Option<TSharkPacket> {
        match value {
            Value::Object(data) => Some(TSharkPacket { data }),
            _ => None,
        }
    }

    fn get_source_layers(&self) -> Option<&serde_json::Map<String, Value>> {
        self.data
            .get("_source")
            .and_then(|src| src.get("layers"))
            .and_then(Value::as_object)
    }

    pub fn get_ip(&self) -> Option<&serde_json::Map<String, Value>> {
        self.get_source_layers()
            .and_then(|layer| layer.get("ip"))
            .and_then(Value::as_object)
    }

    pub fn get_ip_src(&self) -> Option<&str> {
        self.get_ip()
            .and_then(|ip| ip.get("ip.src"))
            .and_then(Value::as_str)
    }

    pub fn get_tcp(&self) -> Option<&serde_json::Map<String, Value>> {
        self.get_source_layers()
            .and_then(|layer| layer.get("tcp"))
            .and_then(Value::as_object)
    }

    pub fn get_tcp_stream(&self) -> Option<&str> {
        self.get_tcp()
            .and_then(|layer| layer.get("tcp.stream"))
            .and_then(Value::as_str)
    }
}
