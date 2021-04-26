use crate::http::http_message_parser;
use crate::http::http_message_parser::{HttpBody, HttpMessageData, HttpRequestResponseData};
use crate::icons;
use crate::message_parser::{MessageInfo, MessageParser, StreamData};
use crate::tshark_communication::TSharkCommunication;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::win;
use crate::BgFunc;
use std::path::PathBuf;
use std::str;
use std::sync::mpsc;

pub struct Http2;

impl MessageParser for Http2 {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.http2.is_some()
    }

    fn protocol_icon(&self) -> icons::Icon {
        icons::Icon::HTTP
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
        let mut summary_details = None;
        let mut messages = vec![];
        let mut cur_request = None;
        for msg in stream {
            if let Some(http2) = msg.source.layers.http2 {
                let tcp_stream_no = msg.source.layers.tcp.as_ref().unwrap().stream;
                let tcp_seq_number = msg.source.layers.tcp.as_ref().unwrap().seq_number;
                let timestamp = msg.source.layers.frame.frame_time;
                for http2_msg in http2.messages {
                    let body = http2_msg
                        .data
                        .map(|d| {
                            str::from_utf8(&d)
                                .ok()
                                .map(|s| HttpBody::Text(s.to_string()))
                                .unwrap_or_else(|| HttpBody::Binary(d))
                        })
                        .unwrap_or(HttpBody::Missing);
                    let content_type = http_message_parser::get_http_header_value(
                        &http2_msg.headers,
                        "content-type",
                    )
                    .cloned();
                    if http_message_parser::get_http_header_value(&http2_msg.headers, ":status")
                        .is_none()
                    {
                        // every http2 response must contain a ":status" header
                        // https://tools.ietf.org/html/rfc7540#section-8.1.2.4
                        // => this is a request
                        let first_line = format!(
                            "{} {}",
                            http_message_parser::get_http_header_value(
                                &http2_msg.headers,
                                ":method"
                            )
                            .map(|s| s.as_str())
                            .unwrap_or("-"),
                            http_message_parser::get_http_header_value(&http2_msg.headers, ":path")
                                .map(|s| s.as_str())
                                .unwrap_or("-")
                        );
                        cur_request = Some(HttpRequestResponseData {
                            tcp_stream_no,
                            tcp_seq_number,
                            timestamp,
                            first_line,
                            content_type,
                            headers: http2_msg.headers,
                            body,
                        });
                    } else {
                        // this is a response
                        let first_line = format!(
                            "HTTP/2 status {}",
                            http_message_parser::get_http_header_value(
                                &http2_msg.headers,
                                ":status"
                            )
                            .map(|s| s.as_str())
                            .unwrap_or("-"),
                        );
                        let response = Some(HttpRequestResponseData {
                            tcp_stream_no,
                            tcp_seq_number,
                            timestamp,
                            first_line,
                            content_type,
                            headers: http2_msg.headers,
                            body,
                        });
                        messages.push(MessageData::Http(HttpMessageData {
                            request: cur_request,
                            response,
                        }));
                        cur_request = None;
                    }
                }
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

    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[MessageData],
        start_idx: i32,
    ) {
        http_message_parser::Http.populate_treeview(ls, session_id, messages, start_idx)
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)> {
        http_message_parser::Http.add_details_to_scroll(parent, overlay, bg_sender, win_msg_sender)
    }

    fn get_empty_liststore(&self) -> gtk::ListStore {
        http_message_parser::Http.get_empty_liststore()
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) {
        http_message_parser::Http.prepare_treeview(tv);
    }

    fn requests_details_overlay(&self) -> bool {
        http_message_parser::Http.requests_details_overlay()
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        http_message_parser::Http.end_populate_treeview(tv, ls);
    }
}
