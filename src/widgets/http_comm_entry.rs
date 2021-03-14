use crate::icons::Icon;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_remote_server::MessageParser;
use crate::widgets::comm_remote_server::MessageParserDetailsMsg;
use crate::TSharkCommunication;
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

    fn parse_stream(&self, stream: &Vec<TSharkCommunication>) -> Vec<MessageData> {
        stream
            .into_iter()
            .filter_map(HttpMessageData::from_json)
            .map(MessageData::Http)
            .collect()
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) -> gtk::ListStore {
        let liststore = gtk::ListStore::new(&[
            // TODO add: response time, content type, body size...
            String::static_type(), // request first line
            String::static_type(), // response first line
        ]);

        let request_col = gtk::TreeViewColumnBuilder::new().title("Request").build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new().build();
        request_col.pack_start(&cell_r_txt, true);
        request_col.add_attribute(&cell_r_txt, "text", 0);
        tv.append_column(&request_col);

        let response_col = gtk::TreeViewColumnBuilder::new().title("Response").build();
        let cell_resp_txt = gtk::CellRendererTextBuilder::new().build();
        response_col.pack_start(&cell_resp_txt, true);
        response_col.add_attribute(&cell_resp_txt, "text", 1);
        tv.append_column(&response_col);

        tv.set_model(Some(&liststore));

        liststore
    }

    fn populate_treeview(&self, ls: &gtk::ListStore, messages: &Vec<MessageData>) {
        for message in messages {
            let iter = ls.append();
            let http = message.as_http().unwrap();
            ls.set_value(&iter, 0, &http.request_response_first_line.to_value());
            ls.set_value(
                &iter,
                1,
                &"TODO (i'm not merging req/resp yet...)".to_value(),
            );
        }
    }

    fn add_details_to_box(&self, vbox: &gtk::Box) -> relm::StreamHandle<MessageParserDetailsMsg> {
        let component = Box::leak(Box::new(vbox.add_widget::<HttpCommEntry>(
            HttpMessageData {
                request_response_first_line: "".to_string(),
                request_response_other_lines: "".to_string(),
                request_response_body: None,
            },
        )));
        component.stream()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpMessageData {
    pub request_response_first_line: String,
    pub request_response_other_lines: String,
    pub request_response_body: Option<String>,
}

impl HttpMessageData {
    pub fn from_json(comm: &TSharkCommunication) -> Option<HttpMessageData> {
        let serde_json = comm.source.layers.http.as_ref();
        if let Some(serde_json::Value::Object(http_map)) = serde_json {
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
