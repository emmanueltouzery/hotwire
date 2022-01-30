use super::http_details_widget;
use super::http_details_widget::HttpCommEntry;
use crate::colors;
use crate::custom_streams_store;
use crate::custom_streams_store::{ClientServerInfo, CustomStreamsStore};
use crate::http::tshark_http::HttpType;
use crate::icons::Icon;
use crate::search_expr;
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpSeqNumber, TcpStreamId};
use crate::widgets::win;
use crate::BgFunc;
use chrono::NaiveDateTime;
use flate2::read::GzDecoder;
use gtk::prelude::*;
use relm::ContainerWidget;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::prelude::*;
use std::net::IpAddr;
use std::str;
use std::str::FromStr;
use std::sync::mpsc;
use strum::VariantNames;
use strum_macros::{EnumString, EnumVariantNames};

lazy_static! {
    // so, sometimes I see host headers like that: "Host: 10.215.215.9:8081".
    // That's completely useless to give me the real hostname, and if later
    // in the stream I get the real hostname, I ignore it because I "already got"
    // the hostname. So filter out these and ignore them.
    static ref IP_ONLY_CHARS: Vec<char> = "0123456789.:".chars().collect();
}

#[derive(Default)]
pub struct HttpStreamData {
    pub stream_globals: HttpStreamGlobals,
    pub client_server: Option<ClientServerInfo>,
    pub messages: Vec<HttpMessageData>,
    pub summary_details: Option<String>,
}

#[derive(Default)]
pub struct HttpStreamsStore {
    streams: HashMap<TcpStreamId, HttpStreamData>,
    component: Option<relm::Component<HttpCommEntry>>,
}

impl HttpStreamsStore {
    fn get_msg_info(
        &self,
        stream_id: TcpStreamId,
        msg_idx: usize,
    ) -> Option<(&HttpMessageData, ClientServerInfo)> {
        let stream = self.streams.get(&stream_id)?;
        let msg = stream.messages.get(msg_idx)?;
        Some((msg, stream.client_server?))
    }
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

#[derive(EnumString, EnumVariantNames)]
pub enum HttpFilterKeys {
    #[strum(serialize = "http.req_line")]
    ReqLine,
    #[strum(serialize = "http.resp_status")]
    RespStatus,
    #[strum(serialize = "http.req_content_type")]
    ReqContentType,
    #[strum(serialize = "http.resp_content_type")]
    RespContentType,
    #[strum(serialize = "http.req_header")]
    ReqHeader,
    #[strum(serialize = "http.resp_header")]
    RespHeader,
    #[strum(serialize = "http.req_body")]
    ReqBody,
    #[strum(serialize = "http.resp_body")]
    RespBody,
}

fn get_http_message<'a, 'b>(
    streams: &'a HashMap<TcpStreamId, &Vec<HttpMessageData>>,
    model: &'b gtk::TreeModel,
    iter: &'b gtk::TreeIter,
) -> Option<&'a HttpMessageData> {
    let (stream_id, idx) = custom_streams_store::get_message_helper(model, iter);
    streams.get(&stream_id).and_then(|s| s.get(idx as usize))
}

impl CustomStreamsStore for HttpStreamsStore {
    fn is_my_message(&self, msg: &TSharkPacket) -> bool {
        msg.http.is_some()
    }

    fn tshark_filter_string(&self) -> &'static str {
        "http"
    }

    fn protocol_icon(&self) -> Icon {
        Icon::HTTP
    }

    fn protocol_name(&self) -> &'static str {
        "HTTP"
    }

    fn reset(&mut self) {
        self.streams = HashMap::new();
    }

    fn stream_message_count(&self, stream_id: TcpStreamId) -> Option<usize> {
        self.streams.get(&stream_id).map(|s| s.messages.len())
    }

    fn stream_summary_details(&self, stream_id: TcpStreamId) -> Option<&str> {
        self.streams
            .get(&stream_id)
            .and_then(|s| s.summary_details.as_deref())
    }

    fn stream_client_server(&self, stream_id: TcpStreamId) -> Option<ClientServerInfo> {
        self.streams.get(&stream_id).and_then(|s| s.client_server)
    }

    fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    fn tcp_stream_ids(&self) -> Vec<TcpStreamId> {
        self.streams.keys().copied().collect()
    }

    fn has_stream_id(&self, stream_id: TcpStreamId) -> bool {
        self.streams.contains_key(&stream_id)
    }

    fn add_to_stream(
        &mut self,
        stream_id: TcpStreamId,
        new_packet: TSharkPacket,
    ) -> Result<Option<ClientServerInfo>, String> {
        let stream = self
            .streams
            .entry(stream_id)
            .or_insert_with(HttpStreamData::default);
        let rr = parse_request_response(
            new_packet,
            stream.client_server.as_ref().map(|cs| cs.server_ip),
        );
        // the localhost is to discard eg localhost:8080
        let host = rr.host.as_deref();
        let host_without_leading_localhost =
            if let Some(no_localhost) = host.and_then(|h| h.strip_prefix("localhost")) {
                Some(no_localhost)
            } else {
                host
            };
        if stream.summary_details.is_none() {
            match (
                rr.req_resp
                    .data()
                    .and_then(|d| get_http_header_value(&d.headers, "X-Forwarded-Server")),
                host_without_leading_localhost,
                rr.req_resp
                    .data()
                    .and_then(|d| get_http_header_value(&d.headers, "Server")),
            ) {
                (Some(fwd), _, _) => stream.summary_details = Some(fwd.clone()),
                (_, Some(host), _) if !host.trim_end_matches(&IP_ONLY_CHARS[..]).is_empty() => {
                    stream.summary_details = Some(host.to_string())
                }
                (_, _, Some(server)) if stream.stream_globals.server_info.is_none() => {
                    stream.stream_globals.server_info = Some(server.to_string());
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
                stream.stream_globals.cur_request = Some(r);
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
                stream.messages.push(HttpMessageData {
                    http_stream_id: 0,
                    request: stream.stream_globals.cur_request.take(),
                    response: Some(r),
                });
            }
            ReqRespInfo {
                req_resp: RequestOrResponseOrOther::Other,
                ..
            } => {}
        };
        Ok(stream.client_server)
    }

    fn finish_stream(&mut self, stream_id: TcpStreamId) -> Result<(), String> {
        let mut stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or("No data for stream")?;
        let globals = std::mem::take(&mut stream.stream_globals);
        if let Some(req) = globals.cur_request {
            stream.messages.push(HttpMessageData {
                http_stream_id: 0,
                request: Some(req),
                response: None,
            });
        }
        if stream.summary_details.is_none() && stream.stream_globals.server_info.is_some() {
            // don't have summary details, let's use server info as "fallback"
            stream.summary_details = globals.server_info;
        }
        Ok(())
    }

    fn get_empty_liststore(&self) -> gtk::ListStore {
        http_get_empty_liststore()
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) {
        http_prepare_treeview(tv);
    }

    fn populate_treeview(&self, ls: &gtk::ListStore, session_id: TcpStreamId, start_idx: i32) {
        let messages = &self.streams.get(&session_id).unwrap().messages;
        http_populate_treeview(messages, ls, session_id, start_idx);
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        http_end_populate_treeview(tv, ls);
    }

    fn supported_filter_keys(&self) -> &'static [&'static str] {
        HttpFilterKeys::VARIANTS
    }

    fn matches_filter(
        &self,
        filter: &search_expr::SearchOpExpr,
        model: &gtk::TreeModel,
        iter: &gtk::TreeIter,
    ) -> bool {
        http_matches_filter(
            &self
                .streams
                .iter()
                .map(|(k, v)| (*k, &v.messages))
                .collect(),
            filter,
            model,
            iter,
        )
    }

    fn requests_details_overlay(&self) -> bool {
        http_requests_details_overlay()
    }

    fn add_details_to_scroll(
        &mut self,
        parent: &gtk::ScrolledWindow,
        overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) {
        let component = parent.add_widget::<HttpCommEntry>((
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
        ));
        self.component = Some(component);
    }

    fn display_in_details_widget(
        &self,
        bg_sender: mpsc::Sender<BgFunc>,
        stream_id: TcpStreamId,
        msg_idx: usize,
    ) {
        if let Some((http_msg, client_server)) = self.get_msg_info(stream_id, msg_idx) {
            self.component.as_ref().unwrap().stream().emit(
                http_details_widget::Msg::DisplayDetails(
                    bg_sender,
                    client_server.client_ip,
                    stream_id,
                    http_msg.clone(),
                ),
            )
        }
    }
}

pub fn http_matches_filter(
    streams: &HashMap<TcpStreamId, &Vec<HttpMessageData>>,
    filter: &search_expr::SearchOpExpr,
    model: &gtk::TreeModel,
    iter: &gtk::TreeIter,
) -> bool {
    let filter_val = &filter.filter_val.to_lowercase();
    if let Ok(filter_key) = HttpFilterKeys::from_str(filter.filter_key) {
        match filter_key {
            HttpFilterKeys::ReqLine => {
                model
                    .value(iter, 0) // req info
                    .get::<&str>()
                    .unwrap()
                    .to_lowercase()
                    .contains(filter_val)
            }
            HttpFilterKeys::RespStatus => {
                model
                    .value(iter, 1) // resp info
                    .get::<&str>()
                    .unwrap()
                    .to_lowercase()
                    .contains(filter_val)
            }
            HttpFilterKeys::ReqContentType => {
                model
                    .value(iter, 8) // req content type
                    .get::<&str>()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(filter_val)
            }
            HttpFilterKeys::RespContentType => {
                model
                    .value(iter, 9) // resp content type
                    .get::<&str>()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(filter_val)
            }
            HttpFilterKeys::ReqHeader => {
                get_http_message(streams, model, iter).map_or(false, |http_msg| {
                    http_msg
                        .request
                        .as_ref()
                        .filter(|r| {
                            r.headers.iter().any(|(k, v)| {
                                k.to_lowercase().contains(filter_val)
                                    || v.to_lowercase().contains(filter_val)
                            })
                        })
                        .is_some()
                })
            }
            HttpFilterKeys::RespHeader => {
                get_http_message(streams, model, iter).map_or(false, |http_msg| {
                    http_msg
                        .response
                        .as_ref()
                        .filter(|r| {
                            r.headers.iter().any(|(k, v)| {
                                k.to_lowercase().contains(filter_val)
                                    || v.to_lowercase().contains(filter_val)
                            })
                        })
                        .is_some()
                })
            }
            HttpFilterKeys::ReqBody => {
                get_http_message(streams, model, iter).map_or(false, |http_msg| {
                    http_msg
                        .request
                        .as_ref()
                        .filter(|r| {
                            r.body_as_str()
                                .filter(|b| b.to_lowercase().contains(filter_val))
                                .is_some()
                        })
                        .is_some()
                })
            }
            HttpFilterKeys::RespBody => {
                get_http_message(streams, model, iter).map_or(false, |http_msg| {
                    http_msg
                        .response
                        .as_ref()
                        .filter(|r| {
                            r.body_as_str()
                                .filter(|b| b.to_lowercase().contains(filter_val))
                                .is_some()
                        })
                        .is_some()
                })
            }
        }
    } else {
        true
    }
}

pub fn http_requests_details_overlay() -> bool {
    true
}

pub fn http_prepare_treeview(tv: &gtk::TreeView) {
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
    response_col.add_attribute(&cell_resp_txt, "foreground", 12);
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

pub fn http_get_empty_liststore() -> gtk::ListStore {
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
        String::static_type(), // http response color
    ])
}

pub fn http_end_populate_treeview(tv: &gtk::TreeView, ls: &gtk::ListStore) {
    let model_sort = gtk::TreeModelSort::new(ls);
    model_sort.set_sort_column_id(gtk::SortColumn::Index(5), gtk::SortType::Ascending);
    tv.set_model(Some(&model_sort));
}

pub fn http_populate_treeview(
    messages: &[HttpMessageData],
    ls: &gtk::ListStore,
    session_id: TcpStreamId,
    start_idx: i32,
) {
    for (idx, http) in messages.iter().enumerate() {
        let iter = ls.append();
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
        ls.set_value(
            &iter,
            custom_streams_store::TREE_STORE_STREAM_ID_COL_IDX,
            &session_id.as_u32().to_value(),
        );
        ls.set_value(
            &iter,
            custom_streams_store::TREE_STORE_MESSAGE_INDEX_COL_IDX,
            &(start_idx + idx as i32).to_value(),
        );
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
                    &format!("{} ms", (rs.timestamp - rq.timestamp).num_milliseconds()).to_value(),
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
        let str_is_numbers_only = |s: &&str| s.chars().all(|c| c.is_numeric());
        let resp_code: Option<u16> = http
            .response
            .as_ref()
            .and_then(|r| {
                r.first_line
                    .split_ascii_whitespace()
                    .find(str_is_numbers_only)
            })
            .and_then(|s| s.parse().ok());
        ls.set_value(
            &iter,
            12,
            &match resp_code {
                Some(r) if (400..500).contains(&r) => colors::WARNING_COLOR.to_value(),
                Some(r) if (500..600).contains(&r) => colors::ERROR_COLOR.to_value(),
                _ => None::<&str>.to_value(),
            },
        );
    }
}

pub fn parse_headers(other_lines: &str) -> Vec<(String, String)> {
    other_lines
        .lines()
        .filter_map(|l| l.split_once(": "))
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
            (HttpBody::Text(s), _) => Some(Cow::Borrowed(s)), // tshark will do some decoding for us... could have text even if the encoding is gzip
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

fn parse_body(body: Option<Vec<u8>>, _headers: &[(String, String)]) -> HttpBody {
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
