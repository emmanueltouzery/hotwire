use crate::icons::Icon;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_remote_server::MessageParser;
use crate::widgets::comm_remote_server::MessageParserDetailsMsg;
use crate::TSharkCommunication;
use chrono::NaiveDateTime;
use gtk::prelude::*;
use relm::{ContainerWidget, Widget};
use relm_derive::{widget, Msg};

pub struct Http;

impl MessageParser for Http {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.http.is_some()
    }

    fn protocol_icon(&self) -> Icon {
        Icon::HTTP
    }

    fn parse_stream(&self, stream: &[TSharkCommunication]) -> Vec<MessageData> {
        let mut cur_request = None;
        let mut result = vec![];
        for msg in stream {
            match parse_request_response(msg) {
                RequestOrResponseOrOther::Request(r) => {
                    cur_request = Some(r);
                }
                RequestOrResponseOrOther::Response(r) => {
                    result.push(MessageData::Http(HttpMessageData {
                        request: cur_request,
                        response: Some(r),
                    }));
                    cur_request = None;
                }
                RequestOrResponseOrOther::Other => {}
            }
        }
        if let Some(r) = cur_request {
            result.push(MessageData::Http(HttpMessageData {
                request: Some(r),
                response: None,
            }));
        }
        result
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) -> gtk::ListStore {
        let liststore = gtk::ListStore::new(&[
            // TODO add: response time, content type, body size...
            String::static_type(), // request first line
            String::static_type(), // response first line
            u32::static_type(),    // stream_id
            u32::static_type(),    // index of the comm in the model vector
        ]);

        let request_col = gtk::TreeViewColumnBuilder::new()
            .title("Request")
            .expand(true)
            .resizable(true)
            .build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new().build();
        request_col.pack_start(&cell_r_txt, true);
        request_col.add_attribute(&cell_r_txt, "text", 0);
        tv.append_column(&request_col);

        let response_col = gtk::TreeViewColumnBuilder::new()
            .title("Response")
            .resizable(true)
            .build();
        let cell_resp_txt = gtk::CellRendererTextBuilder::new().build();
        response_col.pack_start(&cell_resp_txt, true);
        response_col.add_attribute(&cell_resp_txt, "text", 1);
        tv.append_column(&response_col);

        tv.set_model(Some(&liststore));

        liststore
    }

    fn populate_treeview(&self, ls: &gtk::ListStore, session_id: u32, messages: &Vec<MessageData>) {
        for (idx, message) in messages.iter().enumerate() {
            let iter = ls.append();
            let http = message.as_http().unwrap();
            ls.set_value(
                &iter,
                0,
                &http
                    .request
                    .as_ref()
                    .map(|r| r.first_line.as_str())
                    .unwrap_or("Missing request info")
                    .to_value(),
            );
            ls.set_value(
                &iter,
                1,
                &http
                    .response
                    .as_ref()
                    .map(|r| r.first_line.as_str())
                    .unwrap_or("Missing response info")
                    .to_value(),
            );
            ls.set_value(&iter, 2, &session_id.to_value());
            ls.set_value(&iter, 3, &(idx as u32).to_value());
        }
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
    ) -> relm::StreamHandle<MessageParserDetailsMsg> {
        let component = Box::leak(Box::new(parent.add_widget::<HttpCommEntry>(
            HttpMessageData {
                request: None,
                response: None,
            },
        )));
        component.stream()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequestData {
    pub timestamp: NaiveDateTime,
    pub first_line: String,
    pub other_lines: String,
    pub body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponseData {
    pub timestamp: NaiveDateTime,
    pub first_line: String,
    pub other_lines: String,
    pub body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpMessageData {
    pub request: Option<HttpRequestData>,
    pub response: Option<HttpResponseData>,
}

enum RequestOrResponseOrOther {
    Request(HttpRequestData),
    Response(HttpResponseData),
    Other,
}

fn parse_request_response(comm: &TSharkCommunication) -> RequestOrResponseOrOther {
    let serde_json = comm.source.layers.http.as_ref();
    if let Some(serde_json::Value::Object(http_map)) = serde_json {
        let body = http_map
            .get("http.file_data")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string());
        let extract_first_line = |key_name| {
            http_map
                .iter()
                .find(|(_k, v)| {
                    matches!(
                        v,
                        serde_json::Value::Object(fields) if fields.contains_key(key_name))
                })
                .map(|(k, _v)| k.as_str())
                .unwrap_or("")
                .trim_end_matches("\\r\\n")
                .to_string()
        };
        if let Some(req_line) = http_map.get("http.request.line") {
            return RequestOrResponseOrOther::Request(HttpRequestData {
                timestamp: comm.source.layers.frame.frame_time,
                first_line: extract_first_line("http.request.method"),
                other_lines: itertools::free::join(
                    req_line
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap()),
                    "",
                ),
                body,
            });
        }
        if let Some(resp_line) = http_map.get("http.response.line") {
            return RequestOrResponseOrOther::Response(HttpResponseData {
                timestamp: comm.source.layers.frame.frame_time,
                first_line: extract_first_line("http.response.code"),
                other_lines: itertools::free::join(
                    resp_line
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap()),
                    "",
                ),
                body,
            });
        }
    }
    RequestOrResponseOrOther::Other
}

pub struct Model {
    data: HttpMessageData,
}

#[widget]
impl Widget for HttpCommEntry {
    fn model(relm: &relm::Relm<Self>, data: HttpMessageData) -> Model {
        Model { data }
    }

    fn update(&mut self, event: MessageParserDetailsMsg) {
        match event {
            MessageParserDetailsMsg::DisplayDetails(MessageData::Http(msg)) => {
                self.model.data = msg;
            }
            _ => {}
        }
    }

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            margin_top: 10,
            margin_bottom: 10,
            margin_start: 10,
            margin_end: 10,
            spacing: 10,
            #[style_class="http_first_line"]
            gtk::Label {
                label: &self.model.data.request.as_ref().map(|r| r.first_line.as_str()).unwrap_or("Missing request info"),
                xalign: 0.0
            },
            gtk::Label {
                label: &self.model.data.request.as_ref().map(|r| r.other_lines.as_str()).unwrap_or(""),
                xalign: 0.0
            },
            gtk::Label {
                label: self.model.data.request.as_ref().and_then(|r| r.body.as_ref()).map(|b| b.as_str()).unwrap_or(""),
                xalign: 0.0,
                visible: self.model.data.request.as_ref().and_then(|r| r.body.as_ref()).is_some()
            },
            gtk::Separator {},
            #[style_class="http_first_line"]
            gtk::Label {
                label: &self.model.data.response.as_ref().map(|r| r.first_line.as_str()).unwrap_or("Missing response info"),
                xalign: 0.0
            },
            gtk::Label {
                label: &self.model.data.response.as_ref().map(|r| r.other_lines.as_str()).unwrap_or(""),
                xalign: 0.0
            },
            gtk::Label {
                label: self.model.data.response.as_ref().and_then(|r| r.body.as_ref()).map(|b| b.as_str()).unwrap_or(""),
                xalign: 0.0,
                visible: self.model.data.response.as_ref().and_then(|r| r.body.as_ref()).is_some()
            },
        }
    }
}
