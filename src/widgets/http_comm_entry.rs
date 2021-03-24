use crate::icons::Icon;
use crate::tshark_communication::{HttpType, TSharkHttp};
use crate::tshark_communication_raw::TSharkCommunicationRaw;
use crate::widgets::comm_remote_server::{
    MessageData, MessageParser, MessageParserDetailsMsg, StreamData,
};
use crate::widgets::win;
use crate::BgFunc;
use crate::TSharkCommunication;
use chrono::NaiveDateTime;
use gdk_pixbuf::prelude::*;
use gtk::prelude::*;
use relm::{ContainerWidget, Widget};
use relm_derive::widget;
use std::path::Path;
use std::sync::mpsc;

pub struct Http;

const TEXT_CONTENTS_STACK_NAME: &str = "text";
const IMAGE_CONTENTS_STACK_NAME: &str = "image";

impl MessageParser for Http {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.http.is_some()
    }

    fn protocol_icon(&self) -> Icon {
        Icon::HTTP
    }

    fn parse_stream(&self, stream: Vec<TSharkCommunication>) -> StreamData {
        let mut cur_request = None;
        let mut messages = vec![];
        let mut summary_details = None;
        for msg in stream {
            if summary_details.is_none() {
                if let Some(h) = msg
                    .source
                    .layers
                    .http
                    .as_ref()
                    .and_then(|h| h.http_host.as_ref())
                {
                    summary_details = Some(h.clone());
                }
            }
            match parse_request_response(msg) {
                RequestOrResponseOrOther::Request(r) => {
                    cur_request = Some(r);
                }
                RequestOrResponseOrOther::Response(r) => {
                    messages.push(MessageData::Http(HttpMessageData {
                        request: cur_request,
                        response: Some(r),
                    }));
                    cur_request = None;
                }
                RequestOrResponseOrOther::Other => {}
            }
        }
        if let Some(r) = cur_request {
            messages.push(MessageData::Http(HttpMessageData {
                request: Some(r),
                response: None,
            }));
        }
        StreamData {
            messages,
            summary_details,
        }
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) -> gtk::ListStore {
        let liststore = gtk::ListStore::new(&[
            // TODO add: body size...
            String::static_type(), // request first line
            String::static_type(), // response first line
            u32::static_type(),    // stream_id
            u32::static_type(),    // index of the comm in the model vector
            String::static_type(), // request start timestamp (string)
            i64::static_type(),    // request start timestamp (integer, for sorting)
            i32::static_type(),    // request duration (nanos, for sorting)
            String::static_type(), // request duration display
            String::static_type(), // request content type
            String::static_type(), // response content type
            u32::static_type(),    // tcp sequence number
        ]);

        let timestamp_col = gtk::TreeViewColumnBuilder::new()
            .title("Timestamp")
            .resizable(true)
            .sort_column_id(5)
            .build();
        let cell_t_txt = gtk::CellRendererTextBuilder::new().build();
        timestamp_col.pack_start(&cell_t_txt, true);
        timestamp_col.add_attribute(&cell_t_txt, "text", 4);
        tv.append_column(&timestamp_col);

        let request_col = gtk::TreeViewColumnBuilder::new()
            .title("Request")
            .expand(true)
            .resizable(true)
            .build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new()
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        request_col.pack_start(&cell_r_txt, true);
        request_col.add_attribute(&cell_r_txt, "text", 0);
        tv.append_column(&request_col);

        let response_col = gtk::TreeViewColumnBuilder::new()
            .title("Response")
            .resizable(true)
            .sort_column_id(1) // sort by string.. i could add an integer col with the resp code...
            .build();
        let cell_resp_txt = gtk::CellRendererTextBuilder::new().build();
        response_col.pack_start(&cell_resp_txt, true);
        response_col.add_attribute(&cell_resp_txt, "text", 1);
        tv.append_column(&response_col);

        let duration_col = gtk::TreeViewColumnBuilder::new()
            .title("Duration")
            .resizable(true)
            .sort_column_id(6)
            .build();
        let cell_d_txt = gtk::CellRendererTextBuilder::new().build();
        duration_col.pack_start(&cell_d_txt, true);
        duration_col.add_attribute(&cell_d_txt, "text", 7);
        tv.append_column(&duration_col);

        let response_ct_col = gtk::TreeViewColumnBuilder::new()
            .title("Resp Content type")
            .resizable(true)
            .sort_column_id(9)
            .build();
        let cell_resp_ct_txt = gtk::CellRendererTextBuilder::new().build();
        response_ct_col.pack_start(&cell_resp_ct_txt, true);
        response_ct_col.add_attribute(&cell_resp_ct_txt, "text", 9);
        tv.append_column(&response_ct_col);

        let model_sort = gtk::TreeModelSort::new(&liststore);
        model_sort.set_sort_column_id(gtk::SortColumn::Index(5), gtk::SortType::Ascending);
        tv.set_model(Some(&model_sort));

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
            if let Some(ref rq) = http.request {
                ls.set_value(&iter, 4, &rq.timestamp.to_string().to_value());
                ls.set_value(&iter, 5, &rq.timestamp.timestamp_nanos().to_value());
                if let Some(ref rs) = http.response {
                    ls.set_value(
                        &iter,
                        6,
                        &(rs.timestamp - rq.timestamp).num_milliseconds().to_value(),
                    );
                    ls.set_value(
                        &iter,
                        7,
                        &format!("{} ms", (rs.timestamp - rq.timestamp).num_milliseconds())
                            .to_value(),
                    );
                    ls.set_value(&iter, 8, &rq.content_type.to_value());
                    ls.set_value(&iter, 9, &rs.content_type.to_value());
                    ls.set_value(&iter, 10, &rs.tcp_seq_number.to_value());
                }
            }
        }
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        bg_sender: mpsc::Sender<BgFunc>,
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
    pub tcp_seq_number: u32,
    pub timestamp: NaiveDateTime,
    pub first_line: String,
    pub other_lines: String,
    pub body: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponseData {
    pub tcp_seq_number: u32,
    pub timestamp: NaiveDateTime,
    pub first_line: String,
    pub other_lines: String,
    pub body: Option<String>,
    pub content_type: Option<String>,
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

fn parse_request_response(comm: TSharkCommunication) -> RequestOrResponseOrOther {
    let http = comm.source.layers.http;
    match http.map(|h| (h.http_type, h)) {
        Some((HttpType::Request, h)) => RequestOrResponseOrOther::Request(HttpRequestData {
            tcp_seq_number: comm.source.layers.tcp.as_ref().unwrap().seq_number,
            timestamp: comm.source.layers.frame.frame_time,
            body: h.body,
            first_line: h.first_line,
            other_lines: h.other_lines,
            content_type: h.content_type,
        }),
        Some((HttpType::Response, h)) => RequestOrResponseOrOther::Response(HttpResponseData {
            tcp_seq_number: comm.source.layers.tcp.as_ref().unwrap().seq_number,
            timestamp: comm.source.layers.frame.frame_time,
            body: h.body,
            first_line: h.first_line,
            other_lines: h.other_lines,
            content_type: h.content_type,
        }),
        _ => RequestOrResponseOrOther::Other,
    }
}

pub struct Model {
    data: HttpMessageData,

    _got_image_channel: relm::Channel<Vec<u8>>,
    got_image_sender: relm::Sender<Vec<u8>>,
}

#[widget]
impl Widget for HttpCommEntry {
    fn model(relm: &relm::Relm<Self>, data: HttpMessageData) -> Model {
        let stream = relm.stream().clone();
        let (_got_image_channel, got_image_sender) =
            relm::Channel::new(move |r: Vec<u8>| stream.emit(MessageParserDetailsMsg::GotImage(r)));
        Model {
            data,
            _got_image_channel,
            got_image_sender,
        }
    }

    fn update(&mut self, event: MessageParserDetailsMsg) {
        match event {
            MessageParserDetailsMsg::DisplayDetails(
                bg_sender,
                file_path,
                MessageData::Http(msg),
            ) => {
                match (
                    &msg.response.as_ref().and_then(|r| r.content_type.as_ref()),
                    self.model
                        .data
                        .response
                        .as_ref()
                        .and_then(|r| r.body.as_ref()),
                ) {
                    (Some(content_type), Some(body))
                        if content_type.starts_with("image/") && msg.response.is_some() =>
                    {
                        let seq_no = msg.response.as_ref().unwrap().tcp_seq_number;
                        let s = self.model.got_image_sender.clone();
                        bg_sender
                            .send(BgFunc::new(move || {
                                Self::load_image(&file_path, seq_no, s.clone())
                            }))
                            .unwrap();
                    }
                    _ => {
                        self.widgets
                            .contents_stack
                            .set_visible_child_name(TEXT_CONTENTS_STACK_NAME);
                    }
                }
                self.model.data = msg;
            }
            MessageParserDetailsMsg::GotImage(bytes) => {
                let loader = gdk_pixbuf::PixbufLoader::new();
                loader.write(&bytes).unwrap();
                loader.close().unwrap();
                self.widgets
                    .body_image
                    .set_from_pixbuf(loader.get_pixbuf().as_ref());
                self.widgets
                    .contents_stack
                    .set_visible_child_name(IMAGE_CONTENTS_STACK_NAME);
            }
            _ => {}
        }
    }

    fn load_image(file_path: &Path, tcp_seq_number: u32, s: relm::Sender<Vec<u8>>) {
        let mut packets = win::invoke_tshark::<TSharkCommunicationRaw>(
            file_path,
            win::TSharkMode::JsonRaw,
            &format!("tcp.seq eq {}", tcp_seq_number),
        )
        .expect("tshark error");
        if packets.len() == 1 {
            let bytes = packets.pop().unwrap().source.layers.http.unwrap().file_data;
            s.send(bytes).unwrap();
        } else {
            panic!(format!(
                "unexpected json from tshark, tcp stream {}",
                tcp_seq_number
            ));
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
            #[name="contents_stack"]
            gtk::Stack {
                gtk::Label {
                    child: {
                        name: Some(TEXT_CONTENTS_STACK_NAME)
                    },
                    label: self.model.data.response.as_ref().and_then(|r| r.body.as_ref()).map(|b| b.as_str()).unwrap_or(""),
                    xalign: 0.0,
                    visible: self.model.data.response.as_ref().and_then(|r| r.body.as_ref()).is_some()
                },
                #[name="body_image"]
                gtk::Image {
                    child: {
                        name: Some(IMAGE_CONTENTS_STACK_NAME)
                    },
                }
            }
        }
    }
}
