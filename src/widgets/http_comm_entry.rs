use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Clone)]
pub struct HttpMessageData {
    pub request_response_first_line: String,
    pub request_response_other_lines: String,
    pub request_response_body: Option<String>,
}

impl HttpMessageData {
    pub fn from_json(serde_json: &serde_json::Value) -> Option<HttpMessageData> {
        if let serde_json::Value::Object(http_map) = serde_json {
            http_map.iter().find(|(_,v)| matches!(v,
                        serde_json::Value::Object(fields) if fields.contains_key("http.request.method") || fields.contains_key("http.response.code")
            )).map(|(k,_)| HttpMessageData {
                request_response_first_line: k.trim_end_matches("\\r\\n").to_string(),
                request_response_other_lines: itertools::free::join(
                    http_map.get("http.request.line")
                            .unwrap_or_else(|| http_map.get("http.response.line").unwrap())
                            .as_array().unwrap().iter().map(|v| v.as_str().unwrap()), ""),
                request_response_body: http_map.get("http.file_data").and_then(|v| v.as_str()).map(|v| v.trim().to_string())
            })
        } else {
            None
        }
    }
}

#[derive(Msg)]
pub enum Msg {}

pub struct Model {
    data: HttpMessageData,
}

#[widget]
impl Widget for HttpCommEntry {
    fn model(relm: &relm::Relm<Self>, data: HttpMessageData) -> Model {
        Model { data }
    }

    fn update(&mut self, event: Msg) {}

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            gtk::Separator {},
            #[style_class="http_first_line"]
            gtk::Label {
                label: &self.model.data.request_response_first_line,
                xalign: 0.0
            },
            gtk::Label {
                label: &self.model.data.request_response_other_lines,
                xalign: 0.0
            },
            gtk::Label {
                label: self.model.data.request_response_body.as_deref().unwrap_or(""),
                xalign: 0.0,
                visible: self.model.data.request_response_body.is_some()
            },
        }
    }
}
