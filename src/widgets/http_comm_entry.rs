use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg)]
pub enum Msg {}

#[derive(Clone)]
pub struct HttpMessageData {
    pub request_response_first_line: String,
}

impl HttpMessageData {
    pub fn from_json(serde_json: &serde_json::Value) -> Option<HttpMessageData> {
        if let serde_json::Value::Object(http_map) = serde_json {
            http_map.iter().find(|(_,v)| matches!(v,
                        serde_json::Value::Object(fields) if fields.contains_key("http.request.method") || fields.contains_key("http.response.code")
            )).map(|(k,_)| HttpMessageData { request_response_first_line: k.to_string()})
        } else {
            None
        }
    }
}

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
        gtk::Label {
            label: &self.model.data.request_response_first_line,
            xalign: 0.0
        }
    }
}
