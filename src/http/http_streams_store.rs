use super::http_details_widget;
use super::http_details_widget::HttpCommEntry;
use crate::colors;
use crate::custom_streams_store;
use crate::custom_streams_store::{ClientServerInfo, CustomStreamsStore};
use crate::http::tshark_http::HttpType;
use crate::icons::Icon;
use crate::search_expr;
use crate::search_expr::SearchCriteria;
use crate::tshark_communication::{NetworkPort, TSharkPacket, TcpSeqNumber, TcpStreamId};
use crate::widgets::win;
use crate::BgFunc;
use chrono::NaiveDateTime;
use flate2::read::GzDecoder;
use gtk::prelude::*;
use relm::ContainerWidget;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::prelude::*;
use std::net::IpAddr;
use std::str;
use std::str::FromStr;
use std::sync::mpsc;
use strum::VariantNames;
use strum_macros::{EnumString, EnumVariantNames};

#[cfg(test)]
use {
    crate::custom_streams_store::common_tests_parse_stream,
    crate::tshark_communication::parse_test_xml_no_wrapper, chrono::NaiveDate,
};

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
    cur_requests: VecDeque<HttpRequestResponseData>,

    tcp_leftover_payload: Option<(TcpSeqNumber, NaiveDateTime, Vec<u8>)>,

    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Server
    // will put the description of the Server http header in the summary details
    // if we can't get the hostname. Don't store it directly in the summary details,
    // in case we later get the hostname when parsing later packets.
    // if we didn't get the hostname, we use this in finish_stream() to populate
    // the summary details.
    server_info: Option<String>,
}

impl HttpStreamGlobals {
    fn http_resp_from_tcp_if_any(
        &mut self,
        tcp_stream_id: TcpStreamId,
    ) -> Option<HttpRequestResponseData> {
        self.tcp_leftover_payload
            .take()
            .and_then(|(s, dt, d)| Self::parse_as_http(s, dt, d, tcp_stream_id))
    }

    fn get_headers_body(data: &[u8]) -> Option<(&[u8], &[u8])> {
        let mut idx = 0;
        let mut found = false;
        let crlf2 = [b'\r', b'\n', b'\r', b'\n'];
        while idx + crlf2.len() < data.len() {
            if data[idx..(idx + crlf2.len())] == crlf2 {
                found = true;
                break;
            }
            idx += 1;
        }
        if !found {
            return None;
        }
        Some((&data[0..idx], &data[(idx + crlf2.len())..]))
    }

    fn parse_as_http(
        seq: TcpSeqNumber,
        dt: NaiveDateTime,
        data: Vec<u8>,
        tcp_stream_id: TcpStreamId,
    ) -> Option<HttpRequestResponseData> {
        let (headers, raw_body) = Self::get_headers_body(&data)?;
        let header_lines: Vec<_> = str::from_utf8(headers).ok()?.lines().collect();
        let first_line = header_lines.first()?.to_string();
        let headers = header_lines[1..]
            .iter()
            .map(|l| {
                l.split_once(": ")
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            })
            .collect::<Option<Vec<_>>>()?;
        let content_type = get_http_header_value(&headers, "Content-Type").map(|s| s.to_string());
        let content_encoding = ContentEncoding::parse_from_str(
            &get_http_header_value(&headers, "Content-Encoding").map(|s| s.as_str()),
        );
        let body = match str::from_utf8(raw_body) {
            Ok(txt) => HttpBody::Text(txt.to_string()),
            _ => HttpBody::Binary(raw_body.to_vec()),
        };
        Some(HttpRequestResponseData {
            tcp_stream_no: tcp_stream_id,
            tcp_seq_number: seq,
            timestamp: dt,
            first_line,
            headers,
            content_type,
            content_encoding,
            body,
        })
    }
}

#[derive(EnumString, EnumVariantNames, Debug)]
pub enum HttpNumericFilterKeys {
    #[strum(serialize = "http.req_body_bytes")]
    ReqBodyBytes,
    #[strum(serialize = "http.req_body_kilobytes")]
    ReqBodyKilobytes,
    #[strum(serialize = "http.req_body_megabytes")]
    ReqBodyMegabytes,
    #[strum(serialize = "http.resp_body_bytes")]
    RespBodyBytes,
    #[strum(serialize = "http.resp_body_kilobytes")]
    RespBodyKilobytes,
    #[strum(serialize = "http.resp_body_megabytes")]
    RespBodyMegabytes,
}

#[derive(EnumString, EnumVariantNames, Debug)]
pub enum HttpStringFilterKeys {
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
        // I need TCP messages here too, due to the feature of recovering
        // HTTP messages that wireshark clarified as TCP only
        // see the should_add_non_http_tcp_stream_at_end_of_exchange test
        "http || tcp"
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
        if new_packet.http.is_some() {
            // for now we only try to parse TCP leftover packets at the
            // end of a stream. That's the only case I've seen so far.
            stream.stream_globals.tcp_leftover_payload = None;
        }
        let rrs = parse_request_response(
            new_packet,
            stream
                .client_server
                .as_ref()
                .map(|cs| (cs.server_ip, cs.server_port)),
        );
        for rr in rrs.into_iter() {
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
                    req_resp: RequestOrResponse::Request(r),
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
                    stream.stream_globals.cur_requests.push_back(r);
                }
                ReqRespInfo {
                    req_resp: RequestOrResponse::Response(r),
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
                        request: stream.stream_globals.cur_requests.pop_front(),
                        response: Some(r),
                    });
                }
                ReqRespInfo {
                    req_resp: RequestOrResponse::ResponseBytes(date, seq, bytes),
                    ..
                } => {
                    stream.stream_globals.tcp_leftover_payload = Some(
                        if let Some(mut payload_sofar) =
                            stream.stream_globals.tcp_leftover_payload.take()
                        {
                            payload_sofar.2.extend_from_slice(&bytes);
                            payload_sofar
                        } else {
                            (seq, date, bytes)
                        },
                    );
                }
            };
        }
        Ok(stream.client_server)
    }

    fn finish_stream(&mut self, stream_id: TcpStreamId) -> Result<(), String> {
        let mut stream = self
            .streams
            .get_mut(&stream_id)
            .ok_or("No data for stream")?;
        let mut globals = std::mem::take(&mut stream.stream_globals);
        for req in std::mem::take(&mut globals.cur_requests).into_iter() {
            stream.messages.push(HttpMessageData {
                http_stream_id: 0,
                request: Some(req),
                response: globals.http_resp_from_tcp_if_any(stream_id),
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

    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: TcpStreamId,
        start_idx: usize,
        item_count: usize,
    ) {
        let messages = &self.streams.get(&session_id).unwrap().messages;
        http_populate_treeview(messages, ls, session_id, start_idx, item_count);
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        http_end_populate_treeview(tv, ls);
    }

    fn supported_string_filter_keys(&self) -> &'static [&'static str] {
        HttpStringFilterKeys::VARIANTS
    }

    fn supported_numeric_filter_keys(&self) -> &'static [&'static str] {
        HttpNumericFilterKeys::VARIANTS
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
    match (
        HttpStringFilterKeys::from_str(filter.filter_key),
        HttpNumericFilterKeys::from_str(filter.filter_key),
        &filter.op,
    ) {
        (Ok(filter_key), _, SearchCriteria::Contains(filter_val_orig)) => {
            let filter_val = &filter_val_orig.to_lowercase();
            match filter_key {
                HttpStringFilterKeys::ReqLine => {
                    model
                        .value(iter, 0) // req info
                        .get::<&str>()
                        .unwrap()
                        .to_lowercase()
                        .contains(filter_val)
                }
                HttpStringFilterKeys::RespStatus => {
                    model
                        .value(iter, 1) // resp info
                        .get::<&str>()
                        .unwrap()
                        .to_lowercase()
                        .contains(filter_val)
                }
                HttpStringFilterKeys::ReqContentType => {
                    model
                        .value(iter, 8) // req content type
                        .get::<&str>()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(filter_val)
                }
                HttpStringFilterKeys::RespContentType => {
                    model
                        .value(iter, 9) // resp content type
                        .get::<&str>()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(filter_val)
                }
                HttpStringFilterKeys::ReqHeader => {
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
                HttpStringFilterKeys::RespHeader => {
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
                HttpStringFilterKeys::ReqBody => {
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
                HttpStringFilterKeys::RespBody => {
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
        }
        (_, Ok(filter_key), SearchCriteria::GreaterThan(filter_val, decimals)) => {
            match filter_key {
                HttpNumericFilterKeys::ReqBodyBytes => body_size_matches(
                    streams,
                    model,
                    iter,
                    |m| m.request.as_ref(),
                    *filter_val,
                    *decimals,
                    1,
                ),
                HttpNumericFilterKeys::ReqBodyKilobytes => body_size_matches(
                    streams,
                    model,
                    iter,
                    |m| m.request.as_ref(),
                    *filter_val,
                    *decimals,
                    1024,
                ),
                HttpNumericFilterKeys::ReqBodyMegabytes => body_size_matches(
                    streams,
                    model,
                    iter,
                    |m| m.request.as_ref(),
                    *filter_val,
                    *decimals,
                    1024 * 1024,
                ),
                HttpNumericFilterKeys::RespBodyBytes => body_size_matches(
                    streams,
                    model,
                    iter,
                    |m| m.response.as_ref(),
                    *filter_val,
                    *decimals,
                    1,
                ),
                HttpNumericFilterKeys::RespBodyKilobytes => body_size_matches(
                    streams,
                    model,
                    iter,
                    |m| m.response.as_ref(),
                    *filter_val,
                    *decimals,
                    1024,
                ),
                HttpNumericFilterKeys::RespBodyMegabytes => body_size_matches(
                    streams,
                    model,
                    iter,
                    |m| m.response.as_ref(),
                    *filter_val,
                    *decimals,
                    1024 * 1024,
                ),
            }
        }
        _ => true,
    }
}

fn body_size_matches<F>(
    streams: &HashMap<TcpStreamId, &Vec<HttpMessageData>>,
    model: &gtk::TreeModel,
    iter: &gtk::TreeIter,
    get_req_resp: F,
    filter_val: usize,
    decimals: u8,
    multiplier: i32,
) -> bool
where
    F: Fn(&HttpMessageData) -> Option<&HttpRequestResponseData>,
{
    let filter_bytes_count = filter_val * (multiplier as usize) / 10_usize.pow(decimals.into());
    get_http_message(streams, model, iter).map_or(false, |http_msg| {
        get_req_resp(http_msg)
            .filter(|r| r.body.len() > filter_bytes_count)
            .is_some()
    })
}

pub fn http_requests_details_overlay() -> bool {
    true
}

pub fn http_prepare_treeview(tv: &gtk::TreeView) {
    let streamcolor_col = gtk::builders::TreeViewColumnBuilder::new()
        .title("S")
        .fixed_width(10)
        .sort_column_id(2)
        .build();
    let cell_s_txt = gtk::builders::CellRendererTextBuilder::new().build();
    streamcolor_col.pack_start(&cell_s_txt, true);
    streamcolor_col.add_attribute(&cell_s_txt, "background", 11);
    tv.append_column(&streamcolor_col);

    let timestamp_col = gtk::builders::TreeViewColumnBuilder::new()
        .title("Timestamp")
        .resizable(true)
        .sort_column_id(5)
        .build();
    let cell_t_txt = gtk::builders::CellRendererTextBuilder::new().build();
    timestamp_col.pack_start(&cell_t_txt, true);
    timestamp_col.add_attribute(&cell_t_txt, "text", 4);
    tv.append_column(&timestamp_col);

    let request_col = gtk::builders::TreeViewColumnBuilder::new()
        .title("Request")
        .expand(true)
        .resizable(true)
        .build();
    let cell_r_txt = gtk::builders::CellRendererTextBuilder::new()
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    request_col.pack_start(&cell_r_txt, true);
    request_col.add_attribute(&cell_r_txt, "text", 0);
    tv.append_column(&request_col);

    let response_col = gtk::builders::TreeViewColumnBuilder::new()
        .title("Response")
        .resizable(true)
        .sort_column_id(1) // sort by string.. i could add an integer col with the resp code...
        .build();
    let cell_resp_txt = gtk::builders::CellRendererTextBuilder::new().build();
    response_col.pack_start(&cell_resp_txt, true);
    response_col.add_attribute(&cell_resp_txt, "text", 1);
    response_col.add_attribute(&cell_resp_txt, "foreground", 12);
    tv.append_column(&response_col);

    let duration_col = gtk::builders::TreeViewColumnBuilder::new()
        .title("Duration")
        .resizable(true)
        .sort_column_id(6)
        .build();
    let cell_d_txt = gtk::builders::CellRendererTextBuilder::new().build();
    duration_col.pack_start(&cell_d_txt, true);
    duration_col.add_attribute(&cell_d_txt, "text", 7);
    tv.append_column(&duration_col);

    let response_ct_col = gtk::builders::TreeViewColumnBuilder::new()
        .title("Resp Content type")
        .resizable(true)
        .sort_column_id(9)
        .build();
    let cell_resp_ct_txt = gtk::builders::CellRendererTextBuilder::new().build();
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
    start_idx: usize,
    item_count: usize,
) {
    for (idx, http) in messages.iter().skip(start_idx).take(item_count).enumerate() {
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
            &((start_idx + idx) as i32).to_value(),
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

impl HttpBody {
    fn len(&self) -> usize {
        match &self {
            HttpBody::Text(s) => s.len(),
            HttpBody::Binary(v) => v.len(),
            HttpBody::Missing => 0,
        }
    }
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

impl ContentEncoding {
    pub fn parse_from_str(input: &Option<&str>) -> ContentEncoding {
        match input {
            Some("br") => ContentEncoding::Brotli,
            Some("gzip") => ContentEncoding::Gzip,
            _ => ContentEncoding::Plain,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpMessageData {
    pub http_stream_id: u32, // only used for http2. always 0 for http1
    pub request: Option<HttpRequestResponseData>,
    pub response: Option<HttpRequestResponseData>,
}

#[derive(Debug)]
enum RequestOrResponse {
    Request(HttpRequestResponseData),
    Response(HttpRequestResponseData),
    ResponseBytes(NaiveDateTime, TcpSeqNumber, Vec<u8>),
}

impl RequestOrResponse {
    fn data(&self) -> Option<&HttpRequestResponseData> {
        match &self {
            RequestOrResponse::Request(r) => Some(r),
            RequestOrResponse::Response(r) => Some(r),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct ReqRespInfo {
    req_resp: RequestOrResponse,
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

fn parse_request_response(
    comm: TSharkPacket,
    server_ip_port_if_known: Option<(IpAddr, NetworkPort)>,
) -> Vec<ReqRespInfo> {
    let mut reqresp = vec![];
    let ip_src = comm.basic_info.ip_src;
    let ip_dst = comm.basic_info.ip_dst;
    let port_src = comm.basic_info.port_src;
    let port_dst = comm.basic_info.port_dst;
    let ip_port_req_resp_guess = match server_ip_port_if_known {
        Some(srv_ip_port) if srv_ip_port == (ip_dst, port_dst) => Some(HttpType::Request),
        Some(srv_ip_port) if srv_ip_port == (ip_src, port_src) => Some(HttpType::Response),
        _ => None,
    };
    match comm.http {
        Some(http_list) => {
            for http in http_list.into_iter() {
                let http_headers = parse_headers(&http.other_lines);
                let http_type = http.http_type.or(ip_port_req_resp_guess);
                match http_type {
                    Some(HttpType::Request) => reqresp.push(ReqRespInfo {
                        req_resp: RequestOrResponse::Request(HttpRequestResponseData {
                            tcp_stream_no: comm.basic_info.tcp_stream_id,
                            tcp_seq_number: comm.basic_info.tcp_seq_number,
                            timestamp: comm.basic_info.frame_time,
                            body: parse_body(http.body, &http_headers),
                            first_line: http.first_line,
                            headers: http_headers,
                            content_type: http.content_type,
                            content_encoding: ContentEncoding::Plain, // not sure whether maybe tshark decodes before us...
                        }),
                        port_dst: comm.basic_info.port_dst,
                        ip_dst,
                        ip_src,
                        host: http.http_host,
                    }),
                    Some(HttpType::Response) => reqresp.push(ReqRespInfo {
                        req_resp: RequestOrResponse::Response(HttpRequestResponseData {
                            tcp_stream_no: comm.basic_info.tcp_stream_id,
                            tcp_seq_number: comm.basic_info.tcp_seq_number,
                            timestamp: comm.basic_info.frame_time,
                            body: parse_body(http.body, &http_headers),
                            first_line: http.first_line,
                            headers: http_headers,
                            content_type: http.content_type,
                            content_encoding: ContentEncoding::Plain, // not sure whether maybe tshark decodes before us...
                        }),
                        port_dst: comm.basic_info.port_src,
                        ip_dst: ip_src,
                        ip_src: ip_dst,
                        host: http.http_host,
                    }),
                    _ => {
                        // couldn't determine whether request or response, ignore
                        // I didn't see real-world captures hitting that case
                        eprintln!("HTTP: couldn't determine whether packet is request or response, ignoring");
                    }
                }
            }
        }
        None => {
            // no http section
            if let (Some(HttpType::Response), Some(payload)) =
                (ip_port_req_resp_guess, comm.tcp_payload)
            {
                reqresp.push(ReqRespInfo {
                    req_resp: RequestOrResponse::ResponseBytes(
                        comm.basic_info.frame_time,
                        comm.basic_info.tcp_seq_number,
                        payload,
                    ),
                    port_dst: comm.basic_info.port_dst,
                    ip_dst,
                    ip_src,
                    host: None,
                });
            }
        }
    }
    reqresp
}

#[cfg(test)]
fn tests_parse_stream(
    packets: Result<Vec<TSharkPacket>, String>,
) -> Result<Vec<HttpMessageData>, String> {
    let mut parser = HttpStreamsStore::default();
    let sid = common_tests_parse_stream(&mut parser, packets)?;
    Ok(parser.streams.get(&sid).unwrap().messages.clone())
}

#[test]
fn should_parse_plain_request_response() {
    let parsed = tests_parse_stream(parse_test_xml_no_wrapper(
        r#"
      <pdml>
        <packet>
          <proto name="ip">
              <field name="ip.src" show="10.215.215.9" />
              <field name="ip.dst" show="10.215.215.9" />
          </proto>
          <proto name="tcp">
            <field name="tcp.srcport" show="53092" />
            <field name="tcp.dstport" show="80" />
            <field name="tcp.payload" value="504f5354202f6170692f73617665645f6f626a65" />
          </proto>
          <proto name="http">
            <field name="" show="GET /test"></field>
            <field name="http.request.line" showname="Host: 192.168.1.1\r\n" hide="yes" size="22" pos="114" show="Host: 192.168.1.1" value="486f73743a203139322e3136382e38382e3230300d0a"/>
          </proto>
        </packet>

        <packet>
          <proto name="ip">
              <field name="ip.src" show="10.215.215.9" />
              <field name="ip.dst" show="10.215.215.9" />
          </proto>
          <proto name="tcp">
            <field name="tcp.srcport" show="80" />
            <field name="tcp.dstport" show="53092" />
          </proto>
          <proto name="http" showname="Hypertext Transfer Protocol" size="1572423" pos="0">
            <field name="" show="HTTP/1.1 200 OK\r\n" size="17" pos="0" value="485454502f312e3120323030204f4b0d0a"></field>
            <field name="http.response.line" showname="Server: Apache/2.4.29 (Ubuntu)\r\n" hide="yes" size="32" pos="54" show="Server: Apache/2.4.29 (Ubuntu)" value="5365727665723a204170616368652f322e342e323920285562756e7475290d0a"/>
          </proto>
        </packet>

        <packet>
          <proto name="ip">
              <field name="ip.src" show="10.215.215.9" />
              <field name="ip.dst" show="10.215.215.9" />
          </proto>
          <proto name="tcp">
            <field name="tcp.srcport" show="53092" />
            <field name="tcp.dstport" show="80" />
            <field name="tcp.payload" value="504f5354202f6170692f73617665645f6f626a65" />
          </proto>
          <proto name="http">
            <field name="" show="GET /test2"></field>
            <field name="http.request.line" showname="Host: 192.168.1.1\r\n" hide="yes" size="22" pos="114" show="Host: 192.168.1.1" value="486f73743a203139322e3136382e38382e3230300d0a"/>
          </proto>
        </packet>
      </pdml>
        "#,
    ))
    .unwrap();
    let expected = vec![
        HttpMessageData {
            http_stream_id: 0,
            request: Some(HttpRequestResponseData {
                tcp_stream_no: TcpStreamId(0),
                tcp_seq_number: TcpSeqNumber(0),
                timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
                first_line: "GET /test".to_string(),
                headers: vec![("Host".to_string(), "192.168.1.1".to_string())],
                body: HttpBody::Missing,
                content_type: None,
                content_encoding: ContentEncoding::Plain,
            }),
            response: Some(HttpRequestResponseData {
                tcp_stream_no: TcpStreamId(0),
                tcp_seq_number: TcpSeqNumber(0),
                timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
                first_line: "HTTP/1.1 200 OK".to_string(),
                headers: vec![("Server".to_string(), "Apache/2.4.29 (Ubuntu)".to_string())],
                body: HttpBody::Missing,
                content_type: None,
                content_encoding: ContentEncoding::Plain,
            }),
        },
        HttpMessageData {
            http_stream_id: 0,
            request: Some(HttpRequestResponseData {
                tcp_stream_no: TcpStreamId(0),
                tcp_seq_number: TcpSeqNumber(0),
                timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
                first_line: "GET /test2".to_string(),
                headers: vec![("Host".to_string(), "192.168.1.1".to_string())],
                body: HttpBody::Missing,
                content_type: None,
                content_encoding: ContentEncoding::Plain,
            }),
            response: None,
        },
    ];
    assert_eq!(expected, parsed);
}

// Sometimes wireshark will not recognize the last HTTP message of a stream as HTTP
// It will be marked as TCP only. So we try to recognize it and parse the tcp
// payload as HTTP in these cases.
// https://serverfault.com/questions/377147/why-wireshark-does-not-recognize-this-http-response
#[test]
fn should_add_non_http_tcp_stream_at_end_of_exchange() {
    let parsed = tests_parse_stream(parse_test_xml_no_wrapper(
        r#"
      <pdml>
        <packet>
          <proto name="ip">
              <field name="ip.src" show="10.215.215.9" />
              <field name="ip.dst" show="10.215.215.9" />
          </proto>
          <proto name="tcp">
            <field name="tcp.srcport" show="53092" />
            <field name="tcp.dstport" show="80" />
            <field name="tcp.payload" value="504f5354202f6170692f73617665645f6f626a65" />
          </proto>
          <proto name="http">
            <field name="" show="POST /test"></field>
            <field name="http.request.line" showname="Host: 192.168.1.1\r\n" hide="yes" size="22" pos="114" show="Host: 192.168.1.1" value="486f73743a203139322e3136382e38382e3230300d0a"/>
          </proto>
        </packet>

        <packet>
          <proto name="ip">
              <field name="ip.src" show="10.215.215.9" />
              <field name="ip.dst" show="10.215.215.9" />
          </proto>
          <proto name="tcp">
            <field name="tcp.srcport" show="80" />
            <field name="tcp.dstport" show="53092" />
            <field name="tcp.payload" value="485454502F312E3120323030204F4B0A436F6E74656E742D547970653A206170706C69636174696F6E2F6E646A736F6E0A436F6E6E656374696F6E3A206B6565702D616C6976650D0A0D0A7B2261747472696275746573223A7B226465736372697074696F6E223A22222C226B6962616E6153617665644F626A6563744D657461223A7B22736561726368536F757263654A534F4E223A227B5C2266696C7465725C223A5B5D2C5C2271756572795C223A7B5C226C616E67756167655C223A5C226B756572795C222C5C2271756572795C223A5C225C227D7D227D2C227469746C65223A2253797374656D204E617669676174" />
          </proto>
        </packet>
      </pdml>
        "#,
    ))
    .unwrap();
    let expected = vec![HttpMessageData {
        http_stream_id: 0,
        request: Some(HttpRequestResponseData {
            tcp_stream_no: TcpStreamId(0),
            tcp_seq_number: TcpSeqNumber(0),
            timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
            first_line: "POST /test".to_string(),
            headers: vec![("Host".to_string(), "192.168.1.1".to_string())],
            body: HttpBody::Missing,
            content_type: None,
            content_encoding: ContentEncoding::Plain,
        }),
        response: Some(HttpRequestResponseData {
            tcp_stream_no: TcpStreamId(1),
            tcp_seq_number: TcpSeqNumber(0),
            timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
            first_line: "HTTP/1.1 200 OK".to_string(),
            headers: vec![
                ("Content-Type".to_string(), "application/ndjson".to_string()),
                ("Connection".to_string(), "keep-alive".to_string()),
            ],
            body: HttpBody::Text("{\"attributes\":{\"description\":\"\",\"kibanaSavedObjectMeta\":{\"searchSourceJSON\":\"{\\\"filter\\\":[],\\\"query\\\":{\\\"language\\\":\\\"kuery\\\",\\\"query\\\":\\\"\\\"}}\"},\"title\":\"System Navigat".to_string()),
            content_type: Some("application/ndjson".to_string()),
            content_encoding: ContentEncoding::Plain,
        }),
    }];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_http11_pipelining() {
    let parsed = tests_parse_stream(parse_test_xml_no_wrapper(
        r#"
      <pdml>
        <packet>
          <proto name="ip">
              <field name="ip.src" show="10.215.215.9" />
              <field name="ip.dst" show="10.215.215.9" />
          </proto>
          <proto name="tcp">
            <field name="tcp.srcport" show="53092" />
            <field name="tcp.dstport" show="80" />
            <field name="tcp.payload" value="504f5354202f6170692f73617665645f6f626a65" />
          </proto>
          <proto name="http">
            <field name="" show="GET /test"></field>
            <field name="http.request.line" showname="Host: 192.168.1.1\r\n" hide="yes" size="22" pos="114" show="Host: 192.168.1.1" value="486f73743a203139322e3136382e38382e3230300d0a"/>
          </proto>
          <proto name="http">
            <field name="" show="GET /test2"></field>
            <field name="http.request.line" showname="Host: 192.168.1.1\r\n" hide="yes" size="22" pos="114" show="Host: 192.168.1.1" value="486f73743a203139322e3136382e38382e3230300d0a"/>
          </proto>
        </packet>

        <packet>
          <proto name="ip">
              <field name="ip.src" show="10.215.215.9" />
              <field name="ip.dst" show="10.215.215.9" />
          </proto>
          <proto name="tcp">
            <field name="tcp.srcport" show="80" />
            <field name="tcp.dstport" show="53092" />
          </proto>
          <proto name="http" showname="Hypertext Transfer Protocol" size="1572423" pos="0">
            <field name="" show="HTTP/1.1 200 OK\r\n" size="17" pos="0" value="485454502f312e3120323030204f4b0d0a"></field>
            <field name="http.response.line" showname="Server: Apache/2.4.29 (Ubuntu)\r\n" hide="yes" size="32" pos="54" show="Server: Apache/2.4.29 (Ubuntu)" value="5365727665723a204170616368652f322e342e323920285562756e7475290d0a"/>
          </proto>
        </packet>

        <packet>
          <proto name="ip">
              <field name="ip.src" show="10.215.215.9" />
              <field name="ip.dst" show="10.215.215.9" />
          </proto>
          <proto name="tcp">
            <field name="tcp.srcport" show="80" />
            <field name="tcp.dstport" show="53092" />
          </proto>
          <proto name="http" showname="Hypertext Transfer Protocol" size="1572423" pos="0">
            <field name="" show="HTTP/1.1 201 OK\r\n" size="17" pos="0" value="485454502f312e3120323030204f4b0d0a"></field>
            <field name="http.response.line" showname="Server: Apache/2.4.29 (Ubuntu)\r\n" hide="yes" size="32" pos="54" show="Server: Apache/2.4.29 (Ubuntu)" value="5365727665723a204170616368652f322e342e323920285562756e7475290d0a"/>
          </proto>
        </packet>
      </pdml>
        "#,
    ))
    .unwrap();
    let expected = vec![
        HttpMessageData {
            http_stream_id: 0,
            request: Some(HttpRequestResponseData {
                tcp_stream_no: TcpStreamId(0),
                tcp_seq_number: TcpSeqNumber(0),
                timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
                first_line: "GET /test".to_string(),
                headers: vec![("Host".to_string(), "192.168.1.1".to_string())],
                body: HttpBody::Missing,
                content_type: None,
                content_encoding: ContentEncoding::Plain,
            }),
            response: Some(HttpRequestResponseData {
                tcp_stream_no: TcpStreamId(0),
                tcp_seq_number: TcpSeqNumber(0),
                timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
                first_line: "HTTP/1.1 200 OK".to_string(),
                headers: vec![("Server".to_string(), "Apache/2.4.29 (Ubuntu)".to_string())],
                body: HttpBody::Missing,
                content_type: None,
                content_encoding: ContentEncoding::Plain,
            }),
        },
        HttpMessageData {
            http_stream_id: 0,
            request: Some(HttpRequestResponseData {
                tcp_stream_no: TcpStreamId(0),
                tcp_seq_number: TcpSeqNumber(0),
                timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
                first_line: "GET /test2".to_string(),
                headers: vec![("Host".to_string(), "192.168.1.1".to_string())],
                body: HttpBody::Missing,
                content_type: None,
                content_encoding: ContentEncoding::Plain,
            }),
            response: Some(HttpRequestResponseData {
                tcp_stream_no: TcpStreamId(0),
                tcp_seq_number: TcpSeqNumber(0),
                timestamp: NaiveDate::from_ymd(1970, 1, 1).and_hms(0, 0, 0),
                first_line: "HTTP/1.1 201 OK".to_string(),
                headers: vec![("Server".to_string(), "Apache/2.4.29 (Ubuntu)".to_string())],
                body: HttpBody::Missing,
                content_type: None,
                content_encoding: ContentEncoding::Plain,
            }),
        },
    ];
    assert_eq!(expected, parsed);
}
