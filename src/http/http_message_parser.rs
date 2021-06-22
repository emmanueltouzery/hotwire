use super::http_details_widget;
use super::http_details_widget::HttpCommEntry;
use crate::colors;
use crate::http::tshark_http::HttpType;
use crate::icons::Icon;
use crate::message_parser::{
    ClientServerInfo, MessageData, MessageInfo, MessageParser, StreamData, StreamGlobals,
};
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpSeqNumber, TcpStreamId};
use crate::widgets::win;
use crate::BgFunc;
use chrono::NaiveDateTime;
use flate2::read::GzDecoder;
use gtk::prelude::*;
use itertools::Itertools; // collect_tuple
use relm::ContainerWidget;
use std::borrow::Cow;
use std::io::prelude::*;
use std::net::IpAddr;
use std::str;
use std::sync::mpsc;

pub struct Http;

lazy_static! {
    // so, sometimes I see host headers like that: "Host: 10.215.215.9:8081".
    // That's completely useless to give me the real hostname, and if later
    // in the stream I get the real hostname, I ignore it because I "already got"
    // the hostname. So filter out these and ignore them.
    static ref IP_ONLY_CHARS: Vec<char> = "0123456789.:".chars().collect();
}

#[derive(Debug, Default)]
pub struct HttpStreamGlobals {
    cur_request: Option<HttpRequestResponseData>,

    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Server
    // will put the description of the Server http header in the summary details
    // if we can't get the hostname. Don't store it directly in the summary details,
    // in case we later get the hostname when parsing later packets.
    // if we didn't get the hostname, we use this in finish_stream() to populate
    // the summary details.
    server_info: Option<String>,
}

impl MessageParser for Http {
    fn is_my_message(&self, msg: &TSharkPacket) -> bool {
        msg.http.is_some()
    }

    fn tshark_filter_string(&self) -> &'static str {
        "http"
    }

    fn protocol_icon(&self) -> Icon {
        Icon::HTTP
    }

    fn initial_globals(&self) -> StreamGlobals {
        StreamGlobals::Http(HttpStreamGlobals::default())
    }

    fn add_to_stream(
        &self,
        mut stream: StreamData,
        new_packet: TSharkPacket,
    ) -> Result<StreamData, String> {
        let mut globals = stream.stream_globals.as_http().unwrap();
        let mut messages = stream.messages;
        let rr = parse_request_response(
            new_packet,
            stream.client_server.as_ref().map(|cs| cs.server_ip),
        );
        if stream.summary_details.is_none() {
            match (
                rr.req_resp
                    .data()
                    .and_then(|d| get_http_header_value(&d.headers, "X-Forwarded-Server")),
                rr.host.as_ref(),
                rr.req_resp
                    .data()
                    .and_then(|d| get_http_header_value(&d.headers, "Server")),
            ) {
                (Some(fwd), _, _) => stream.summary_details = Some(fwd.clone()),
                (_, Some(host), _) if !host.trim_end_matches(&IP_ONLY_CHARS[..]).is_empty() => {
                    stream.summary_details = Some(host.clone())
                }
                (_, _, Some(server)) if globals.server_info.is_none() => {
                    globals.server_info = Some(server.to_string());
                }
                _ => {}
            }
        }
        match rr {
            ReqRespInfo {
                req_resp: RequestOrResponseOrOther::Request(r),
                port_dst: srv_port,
                ip_dst: srv_ip,
                ip_src: cl_ip,
                ..
            } => {
                if stream.client_server.is_none() {
                    stream.client_server = Some(ClientServerInfo {
                        client_ip: cl_ip,
                        server_ip: srv_ip,
                        server_port: srv_port,
                    });
                }
                globals.cur_request = Some(r);
            }
            ReqRespInfo {
                req_resp: RequestOrResponseOrOther::Response(r),
                port_dst: srv_port,
                ip_dst: srv_ip,
                ip_src: cl_ip,
                ..
            } => {
                if stream.client_server.is_none() {
                    stream.client_server = Some(ClientServerInfo {
                        client_ip: cl_ip,
                        server_ip: srv_ip,
                        server_port: srv_port,
                    });
                }
                messages.push(MessageData::Http(HttpMessageData {
                    http_stream_id: 0,
                    request: globals.cur_request.take(),
                    response: Some(r),
                }));
            }
            ReqRespInfo {
                req_resp: RequestOrResponseOrOther::Other,
                ..
            } => {}
        };
        stream.stream_globals = StreamGlobals::Http(globals);
        stream.messages = messages;
        Ok(stream)
    }

    fn finish_stream(&self, mut stream: StreamData) -> Result<StreamData, String> {
        let globals = stream.stream_globals.as_http().unwrap();
        let mut messages = stream.messages;
        if let Some(req) = globals.cur_request {
            messages.push(MessageData::Http(HttpMessageData {
                http_stream_id: 0,
                request: Some(req),
                response: None,
            }));
        }
        if stream.summary_details.is_none() && globals.server_info.is_some() {
            // don't have summary details, let's use server info as "fallback"
            stream.summary_details = globals.server_info;
        }
        stream.stream_globals = StreamGlobals::Http(HttpStreamGlobals::default());
        stream.messages = messages;
        Ok(stream)
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
        session_id: TcpStreamId,
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
            ls.set_value(&iter, 2, &session_id.as_u32().to_value());
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
                    ls.set_value(&iter, 10, &rs.tcp_seq_number.as_u32().to_value());
                }
            }
            ls.set_value(
                &iter,
                11,
                &colors::STREAM_COLORS[session_id.as_u32() as usize % colors::STREAM_COLORS.len()]
                    .to_value(),
            );
        }
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        let model_sort = gtk::TreeModelSort::new(ls);
        model_sort.set_sort_column_id(gtk::SortColumn::Index(5), gtk::SortType::Ascending);
        tv.set_model(Some(&model_sort));
    }

    fn matches_filter(&self, filter: &str, model: &gtk::TreeModel, iter: &gtk::TreeIter) -> bool {
        let filter = &filter.to_lowercase();
        model
            .get_value(iter, 0) // req info
            .get::<&str>()
            .unwrap()
            .unwrap()
            .to_lowercase()
            .contains(filter)
            || model
                .get_value(iter, 1) // resp info
                .get::<&str>()
                .unwrap()
                .unwrap()
                .to_lowercase()
                .contains(filter)
            || model
                .get_value(iter, 8) // req content type
                .get::<&str>()
                .unwrap()
                .unwrap_or("")
                .to_lowercase()
                .contains(filter)
            || model
                .get_value(iter, 9) // resp content type
                .get::<&str>()
                .unwrap()
                .unwrap_or("")
                .to_lowercase()
                .contains(filter)
    }

    fn requests_details_overlay(&self) -> bool {
        true
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)> {
        let component = Box::leak(Box::new(parent.add_widget::<HttpCommEntry>((
            win_msg_sender,
            TcpStreamId(0),
            "0.0.0.0".parse().unwrap(),
            HttpMessageData {
                http_stream_id: 0,
                request: None,
                response: None,
            },
            overlay.unwrap().clone(),
            bg_sender,
        ))));
        Box::new(move |bg_sender, message_info| {
            component
                .stream()
                .emit(http_details_widget::Msg::DisplayDetails(
                    bg_sender,
                    message_info,
                ))
        })
    }
}

pub fn parse_headers(other_lines: &str) -> Vec<(String, String)> {
    other_lines
        .lines()
        .filter_map(|l| {
            // TODO use String.split_once after rust 1.52 is stabilized
            l.splitn(2, ": ").collect_tuple()
        })
        .map(|(k, v)| (k.to_string(), v.trim_end().to_string()))
        .collect()
}

pub fn get_http_header_value<'a>(
    headers: &'a [(String, String)],
    header_name: &str,
) -> Option<&'a String> {
    let lower_header_name = {
        let mut h = String::from(header_name);
        h.make_ascii_lowercase();
        h
    };
    headers.iter().find_map(|(k, v)| {
        let h_name = {
            let mut h_name = String::from(k);
            h_name.make_ascii_lowercase();
            h_name
        };
        if h_name == lower_header_name {
            Some(v)
        } else {
            None
        }
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpBody {
    // TODO the whole binary vs text stuff is possibly obsolete due to content encodings..
    Text(String),
    Binary(Vec<u8>),
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequestResponseData {
    pub tcp_stream_no: TcpStreamId,
    pub tcp_seq_number: TcpSeqNumber,
    pub timestamp: NaiveDateTime,
    pub first_line: String,
    // no hashmap, i want to preserve the order,
    // no btreemap, i want to preserve the case of the keys
    // => can't make a simple lookup anyway
    pub headers: Vec<(String, String)>,
    pub body: HttpBody,
    pub content_type: Option<String>,
    pub content_encoding: ContentEncoding,
}

impl HttpRequestResponseData {
    pub fn body_as_str(&self) -> Option<Cow<str>> {
        match (&self.body, &self.content_encoding) {
            (HttpBody::Text(s), _) => Some(Cow::Borrowed(&s)), // tshark will do some decoding for us... could have text even if the encoding is gzip
            (HttpBody::Binary(bytes), ContentEncoding::Brotli) => {
                let mut r = String::new();
                brotli::Decompressor::new(&bytes[..], 4096)
                    .read_to_string(&mut r)
                    .ok()
                    .map(|_| Cow::Owned(r))
            }
            (HttpBody::Binary(bytes), ContentEncoding::Gzip) => {
                // not sure we really need the gzip. i think tshark always decodes gzip for us
                // (doesn't do it for brotli!!)
                let mut d = GzDecoder::new(&bytes[..]);
                let mut r = String::new();
                d.read_to_string(&mut r).ok().map(|_| Cow::Owned(r))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentEncoding {
    Plain,
    Gzip,
    Brotli,
    // TODO deflate
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpMessageData {
    pub http_stream_id: u32,
    pub request: Option<HttpRequestResponseData>,
    pub response: Option<HttpRequestResponseData>,
}

enum RequestOrResponseOrOther {
    Request(HttpRequestResponseData),
    Response(HttpRequestResponseData),
    Other,
}

impl RequestOrResponseOrOther {
    fn data(&self) -> Option<&HttpRequestResponseData> {
        match &self {
            RequestOrResponseOrOther::Request(r) => Some(r),
            RequestOrResponseOrOther::Response(r) => Some(r),
            _ => None,
        }
    }
}

struct ReqRespInfo {
    req_resp: RequestOrResponseOrOther,
    ip_src: IpAddr,
    port_dst: NetworkPort,
    ip_dst: IpAddr,
    host: Option<String>,
}

fn parse_body(body: Option<Vec<u8>>, headers: &[(String, String)]) -> HttpBody {
    body.map(|d| {
        str::from_utf8(&d)
            .ok()
            .map(|s| HttpBody::Text(s.to_string()))
            .unwrap_or_else(|| HttpBody::Binary(d))
    })
    .unwrap_or(HttpBody::Missing)
}

fn parse_request_response(comm: TSharkPacket, server_ip_if_known: Option<IpAddr>) -> ReqRespInfo {
    let http = comm.http;
    let http_headers = http.as_ref().map(|h| parse_headers(&h.other_lines));
    let ip_src = comm.basic_info.ip_src;
    let ip_dst = comm.basic_info.ip_dst;
    let http_type = http
        .as_ref()
        .and_then(|h| h.http_type)
        .or_else(|| match server_ip_if_known {
            Some(srv_ip) if srv_ip == ip_dst => Some(HttpType::Request),
            Some(srv_ip) if srv_ip == ip_src => Some(HttpType::Response),
            _ => None,
        });
    match http.map(|h| (http_type, h, http_headers)) {
        Some((Some(HttpType::Request), h, Some(headers))) => ReqRespInfo {
            req_resp: RequestOrResponseOrOther::Request(HttpRequestResponseData {
                tcp_stream_no: comm.basic_info.tcp_stream_id,
                tcp_seq_number: comm.basic_info.tcp_seq_number,
                timestamp: comm.basic_info.frame_time,
                body: parse_body(h.body, &headers),
                first_line: h.first_line,
                headers,
                content_type: h.content_type,
                content_encoding: ContentEncoding::Plain, // not sure whether maybe tshark decodes before us...
            }),
            port_dst: comm.basic_info.port_dst,
            ip_dst,
            ip_src,
            host: h.http_host,
        },
        Some((Some(HttpType::Response), h, Some(headers))) => ReqRespInfo {
            req_resp: RequestOrResponseOrOther::Response(HttpRequestResponseData {
                tcp_stream_no: comm.basic_info.tcp_stream_id,
                tcp_seq_number: comm.basic_info.tcp_seq_number,
                timestamp: comm.basic_info.frame_time,
                body: parse_body(h.body, &headers),
                first_line: h.first_line,
                headers,
                content_type: h.content_type,
                content_encoding: ContentEncoding::Plain, // not sure whether maybe tshark decodes before us...
            }),
            port_dst: comm.basic_info.port_src,
            ip_dst: ip_src,
            ip_src: ip_dst,
            host: h.http_host,
        },
        _ => ReqRespInfo {
            req_resp: RequestOrResponseOrOther::Other,
            port_dst: comm.basic_info.port_dst,
            ip_dst,
            ip_src,
            host: None,
        },
    }
}
