use super::comm_info_header;
use super::comm_info_header::CommInfoHeader;
use super::message_parser::{MessageInfo, MessageParser, StreamData};
use crate::colors;
use crate::http::tshark_http::HttpType;
use crate::icons::Icon;
use crate::tshark_communication_raw::TSharkCommunicationRaw;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::win;
use crate::BgFunc;
use crate::TSharkCommunication;
use chrono::NaiveDateTime;
use gdk_pixbuf::prelude::*;
use gtk::prelude::*;
use relm::{ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::path::Path;
use std::path::PathBuf;
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
        let mut server_ip = stream
            .first()
            .unwrap()
            .source
            .layers
            .ip
            .as_ref()
            .unwrap()
            .ip_dst
            .clone();
        let mut server_port = stream
            .first()
            .unwrap()
            .source
            .layers
            .tcp
            .as_ref()
            .unwrap()
            .port_dst;
        let mut cur_request = None;
        let mut messages = vec![];
        let mut summary_details = None;
        for msg in stream {
            if summary_details.is_none() {
                if let Some(h) = msg.source.layers.http.as_ref() {
                    match (
                        get_http_header_value(&h.other_lines, "X-Forwarded-Server"),
                        h.http_host.as_ref(),
                    ) {
                        (Some(fwd), _) => summary_details = Some(fwd.clone()),
                        (_, Some(host)) => summary_details = Some(host.clone()),
                        _ => {}
                    }
                }
            }
            match parse_request_response(msg) {
                (RequestOrResponseOrOther::Request(r), srv_port, srv_ip) => {
                    server_ip = srv_ip;
                    server_port = srv_port;
                    cur_request = Some(r);
                }
                (RequestOrResponseOrOther::Response(r), srv_port, srv_ip) => {
                    server_ip = srv_ip;
                    server_port = srv_port;
                    messages.push(MessageData::Http(HttpMessageData {
                        request: cur_request,
                        response: Some(r),
                    }));
                    cur_request = None;
                }
                (RequestOrResponseOrOther::Other, _, _) => {}
            }
        }
        if let Some(r) = cur_request {
            messages.push(MessageData::Http(HttpMessageData {
                request: Some(r),
                response: None,
            }));
        }
        StreamData {
            server_ip,
            server_port,
            messages,
            summary_details,
        }
    }

    fn get_empty_liststore(&self) -> gtk::ListStore {
        gtk::ListStore::new(&[
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
            String::static_type(), // stream color
        ])
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) {
        let streamcolor_col = gtk::TreeViewColumnBuilder::new()
            .title("S")
            .fixed_width(10)
            .sort_column_id(2)
            .build();
        let cell_s_txt = gtk::CellRendererTextBuilder::new().build();
        streamcolor_col.pack_start(&cell_s_txt, true);
        streamcolor_col.add_attribute(&cell_s_txt, "background", 11);
        tv.append_column(&streamcolor_col);

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
    }

    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[MessageData],
        start_idx: i32,
    ) {
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
            ls.set_value(&iter, 3, &(start_idx + idx as i32).to_value());
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
                    ls.set_value(
                        &iter,
                        11,
                        &colors::STREAM_COLORS[session_id as usize % colors::STREAM_COLORS.len()]
                            .to_value(),
                    );
                }
            }
        }
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        let model_sort = gtk::TreeModelSort::new(ls);
        model_sort.set_sort_column_id(gtk::SortColumn::Index(5), gtk::SortType::Ascending);
        tv.set_model(Some(&model_sort));
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        bg_sender: mpsc::Sender<BgFunc>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)> {
        let component = Box::leak(Box::new(parent.add_widget::<HttpCommEntry>((
            0,
            "".to_string(),
            HttpMessageData {
                request: None,
                response: None,
            },
        ))));
        Box::new(move |bg_sender, path, message_info| {
            component
                .stream()
                .emit(Msg::DisplayDetails(bg_sender, path, message_info))
        })
    }
}

fn get_http_header_value(other_lines: &str, header_name: &str) -> Option<String> {
    // TODO use String.split_once after rust 1.52 is stabilized
    other_lines.lines().find_map(|l| {
        let mut parts = l.splitn(2, ": ");
        if parts.next() == Some(header_name) {
            parts.next().map(|s| s.trim_end().to_string())
        } else {
            None
        }
    })
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

fn parse_request_response(comm: TSharkCommunication) -> (RequestOrResponseOrOther, u32, String) {
    let http = comm.source.layers.http;
    match http.map(|h| (h.http_type, h)) {
        Some((HttpType::Request, h)) => (
            RequestOrResponseOrOther::Request(HttpRequestData {
                tcp_seq_number: comm.source.layers.tcp.as_ref().unwrap().seq_number,
                timestamp: comm.source.layers.frame.frame_time,
                body: h.body,
                first_line: h.first_line,
                other_lines: h.other_lines,
                content_type: h.content_type,
            }),
            comm.source.layers.tcp.as_ref().unwrap().port_dst,
            comm.source
                .layers
                .ip
                .map(|i| i.ip_dst)
                .or(comm.source.layers.ipv6.map(|i| i.ip_dst))
                .unwrap(),
        ),
        Some((HttpType::Response, h)) => (
            RequestOrResponseOrOther::Response(HttpResponseData {
                tcp_seq_number: comm.source.layers.tcp.as_ref().unwrap().seq_number,
                timestamp: comm.source.layers.frame.frame_time,
                body: h.body,
                first_line: h.first_line,
                other_lines: h.other_lines,
                content_type: h.content_type,
            }),
            comm.source.layers.tcp.as_ref().unwrap().port_src,
            comm.source
                .layers
                .ip
                .map(|i| i.ip_src)
                .or(comm.source.layers.ipv6.map(|i| i.ip_src))
                .unwrap(),
        ),
        _ => (
            RequestOrResponseOrOther::Other,
            comm.source.layers.tcp.as_ref().unwrap().port_dst,
            comm.source.layers.ip.unwrap().ip_dst,
        ),
    }
}

#[derive(Msg, Debug)]
pub enum Msg {
    DisplayDetails(mpsc::Sender<BgFunc>, PathBuf, MessageInfo),
    GotImage(Vec<u8>),
}

pub struct Model {
    stream_id: u32,
    client_ip: String,
    data: HttpMessageData,

    _got_image_channel: relm::Channel<Vec<u8>>,
    got_image_sender: relm::Sender<Vec<u8>>,
}

#[widget]
impl Widget for HttpCommEntry {
    fn model(relm: &relm::Relm<Self>, params: (u32, String, HttpMessageData)) -> Model {
        let (stream_id, client_ip, data) = params;
        let stream = relm.stream().clone();
        let (_got_image_channel, got_image_sender) =
            relm::Channel::new(move |r: Vec<u8>| stream.emit(Msg::GotImage(r)));
        Model {
            data,
            stream_id,
            client_ip,
            _got_image_channel,
            got_image_sender,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::DisplayDetails(
                bg_sender,
                file_path,
                MessageInfo {
                    client_ip,
                    stream_id,
                    message_data: MessageData::Http(msg),
                },
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
                self.streams
                    .comm_info_header
                    .emit(comm_info_header::Msg::Update(client_ip.clone(), stream_id));
                self.model.stream_id = stream_id;
                self.model.client_ip = client_ip;
            }
            Msg::GotImage(bytes) => {
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
            #[name="comm_info_header"]
            CommInfoHeader(self.model.client_ip.clone(), self.model.stream_id) {
            },
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
