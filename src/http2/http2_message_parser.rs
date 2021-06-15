use crate::http::http_message_parser;
use crate::http::http_message_parser::{
    ContentEncoding, HttpBody, HttpMessageData, HttpRequestResponseData,
};
use crate::http2::tshark_http2::{Http2Data, TSharkHttp2Message};
use crate::icons;
use crate::message_parser::ClientServerInfo;
use crate::message_parser::{MessageInfo, MessageParser, StreamData};
use crate::tshark_communication::TSharkPacket;
use crate::tshark_communication::TSharkPacketBasicInfo;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_remote_server::StreamGlobals;
use crate::widgets::win;
use crate::BgFunc;
use chrono::NaiveDateTime;
use std::collections::HashMap;
use std::str;
use std::sync::mpsc;

#[cfg(test)]
use crate::tshark_communication::{parse_stream, parse_test_xml};
#[cfg(test)]
use chrono::NaiveDate;

pub struct Http2;

#[derive(Debug, Default)]
pub struct Http2StreamProcessedContents {
    cur_request: Option<HttpRequestResponseData>,
    unfinished_basic_info: Option<TSharkPacketBasicInfo>,
    unfinished_stream_messages: Vec<TSharkHttp2Message>,
}

#[derive(Debug, Default)]
pub struct Http2StreamGlobals {
    pub messages_per_stream: HashMap<u32, Http2StreamProcessedContents>,
}

impl MessageParser for Http2 {
    fn is_my_message(&self, msg: &TSharkPacket) -> bool {
        msg.http2.is_some()
    }

    fn protocol_icon(&self) -> icons::Icon {
        icons::Icon::HTTP
    }

    fn initial_globals(&self) -> StreamGlobals {
        StreamGlobals::Http2(Http2StreamGlobals::default())
    }

    fn add_to_stream(
        &self,
        mut stream: StreamData,
        new_packet: TSharkPacket,
    ) -> Result<StreamData, String> {
        let mut globals = stream.stream_globals.as_http2().unwrap();
        let mut messages = stream.messages;
        let cur_msg = new_packet.basic_info;
        let http2 = new_packet.http2.unwrap();
        for http2_msg in http2 {
            if http2_msg.is_end_stream {
                let http2_stream_id = http2_msg.stream_id;
                // got all the elements of the message, add it to the result
                let mut stream_messages = globals
                    .messages_per_stream
                    .remove(&http2_msg.stream_id)
                    .unwrap_or_else(|| Http2StreamProcessedContents {
                        cur_request: None,
                        unfinished_basic_info: Some(cur_msg),
                        unfinished_stream_messages: vec![],
                    });
                stream_messages.unfinished_stream_messages.push(http2_msg);
                let (http_msg, msg_type) = prepare_http_message(
                    cur_msg.tcp_stream_id,
                    cur_msg.tcp_seq_number,
                    cur_msg.frame_time,
                    stream_messages.unfinished_stream_messages,
                );
                match msg_type {
                    MsgType::Request => {
                        if stream.summary_details.is_none() {
                            stream.summary_details = http_message_parser::get_http_header_value(
                                &http_msg.headers,
                                ":authority",
                            )
                            .map(|c| c.to_string());
                        }
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                client_ip: cur_msg.ip_src,
                                server_ip: cur_msg.ip_dst,
                                server_port: cur_msg.port_dst,
                            });
                        }
                        globals.messages_per_stream.insert(
                            http2_stream_id,
                            Http2StreamProcessedContents {
                                cur_request: Some(http_msg),
                                unfinished_basic_info: Some(cur_msg),
                                unfinished_stream_messages: vec![],
                            },
                        );
                    }
                    MsgType::Response => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                client_ip: cur_msg.ip_dst,
                                server_ip: cur_msg.ip_src,
                                server_port: cur_msg.port_src,
                            });
                        }
                        messages.push(MessageData::Http(HttpMessageData {
                            http_stream_id: http2_stream_id,
                            request: stream_messages.cur_request,
                            response: Some(http_msg),
                        }));
                    }
                }
            } else {
                // collecting more elements for this message
                let stream_msgs_entry = globals
                    .messages_per_stream
                    .entry(http2_msg.stream_id)
                    .or_insert(Http2StreamProcessedContents {
                        cur_request: None,
                        unfinished_basic_info: Some(cur_msg),
                        unfinished_stream_messages: vec![],
                    });
                stream_msgs_entry.unfinished_stream_messages.push(http2_msg);
            }
        }
        stream.stream_globals = StreamGlobals::Http2(globals);
        stream.messages = messages;
        Ok(stream)
    }

    fn finish_stream(&self, mut stream: StreamData) -> Result<StreamData, String> {
        // flush all the incomplete messages as best we can
        let mut globals = stream.stream_globals.as_http2().unwrap();
        let mut messages = stream.messages;
        for (http2_stream_id, stream_contents) in globals.messages_per_stream {
            let cur_msg = stream_contents.unfinished_basic_info.unwrap();
            match (
                stream_contents.cur_request,
                stream_contents.unfinished_stream_messages,
            ) {
                (Some(r), leftover) if leftover.is_empty() => {
                    messages.push(MessageData::Http(HttpMessageData {
                        http_stream_id: http2_stream_id,
                        request: Some(r),
                        response: None,
                    }))
                }
                (req, leftover) => {
                    let (http_msg, msg_type) = prepare_http_message(
                        cur_msg.tcp_stream_id,
                        cur_msg.tcp_seq_number,
                        cur_msg.frame_time,
                        leftover,
                    );
                    match msg_type {
                        MsgType::Request => {
                            if stream.summary_details.is_none() {
                                stream.summary_details =
                                    http_message_parser::get_http_header_value(
                                        &http_msg.headers,
                                        ":authority",
                                    )
                                    .map(|c| c.to_string());
                            }
                            if stream.client_server.is_none() {
                                stream.client_server = Some(ClientServerInfo {
                                    client_ip: cur_msg.ip_src,
                                    server_ip: cur_msg.ip_dst,
                                    server_port: cur_msg.port_dst,
                                });
                            }
                            messages.push(MessageData::Http(HttpMessageData {
                                http_stream_id: http2_stream_id,
                                request: Some(http_msg),
                                response: None,
                            }));
                        }
                        MsgType::Response => {
                            if stream.client_server.is_none() {
                                stream.client_server = Some(ClientServerInfo {
                                    client_ip: cur_msg.ip_dst,
                                    server_ip: cur_msg.ip_src,
                                    server_port: cur_msg.port_src,
                                });
                            }
                            messages.push(MessageData::Http(HttpMessageData {
                                http_stream_id: http2_stream_id,
                                request: req,
                                response: Some(http_msg),
                            }));
                        }
                    }
                }
            }
        }
        stream.stream_globals = StreamGlobals::Http2(Http2StreamGlobals::default());
        stream.messages = messages;
        Ok(stream)
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
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)> {
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

    fn matches_filter(&self, filter: &str, model: &gtk::TreeModel, iter: &gtk::TreeIter) -> bool {
        http_message_parser::Http.matches_filter(filter, model, iter)
    }
}

enum MsgType {
    Request,
    Response,
}

fn prepare_http_message(
    tcp_stream_no: u32,
    tcp_seq_number: u32,
    timestamp: NaiveDateTime,
    http2_msgs: Vec<TSharkHttp2Message>,
) -> (HttpRequestResponseData, MsgType) {
    let (headers, data) = http2_msgs.into_iter().fold(
        (vec![], None::<Vec<u8>>),
        |(mut sofar_h, sofar_d), mut cur| {
            sofar_h.append(&mut cur.headers);
            let new_data = match (sofar_d, cur.data) {
                (None, Some(Http2Data::BasicData(d))) => Some(d),
                (None, Some(Http2Data::RecomposedData(d))) => Some(d),
                (Some(mut s), Some(Http2Data::BasicData(mut n))) => {
                    s.append(&mut n);
                    Some(s)
                }
                (Some(_s), Some(Http2Data::RecomposedData(n))) => Some(n),
                (d, _) => d,
            };
            (sofar_h, new_data)
        },
    );
    let body = data
        .map(|d| {
            str::from_utf8(&d)
                .ok()
                .map(|s| HttpBody::Text(s.to_string()))
                .unwrap_or_else(|| HttpBody::Binary(d))
        })
        .unwrap_or(HttpBody::Missing);

    let (first_line, msg_type) =
        if http_message_parser::get_http_header_value(&headers, ":status").is_none() {
            // every http2 response must contain a ":status" header
            // https://tools.ietf.org/html/rfc7540#section-8.1.2.4
            // => this is a request
            (
                format!(
                    "{} {}",
                    http_message_parser::get_http_header_value(&headers, ":method")
                        .map(|s| s.as_str())
                        .unwrap_or("-"),
                    http_message_parser::get_http_header_value(&headers, ":path")
                        .map(|s| s.as_str())
                        .unwrap_or("-")
                ),
                MsgType::Request,
            )
        } else {
            // this is a response
            (
                format!(
                    "HTTP/2 status {}",
                    http_message_parser::get_http_header_value(&headers, ":status")
                        .map(|s| s.as_str())
                        .unwrap_or("-"),
                ),
                MsgType::Response,
            )
        };
    let content_type =
        http_message_parser::get_http_header_value(&headers, "content-type").cloned();
    let content_encoding =
        match http_message_parser::get_http_header_value(&headers, "content-encoding")
            .map(|s| s.as_str())
        {
            Some("br") => ContentEncoding::Brotli,
            Some("gzip") => ContentEncoding::Gzip,
            _ => ContentEncoding::Plain,
        };
    if matches!(body, HttpBody::Binary(_)) {
        println!(
            "######### GOT BINARY BODY {:?} status {:?} path {:?}",
            content_type,
            http_message_parser::get_http_header_value(&headers, ":status"),
            http_message_parser::get_http_header_value(&headers, ":path"),
        );
    }
    (
        HttpRequestResponseData {
            tcp_stream_no,
            tcp_seq_number,
            timestamp,
            first_line,
            content_type,
            headers,
            body,
            content_encoding,
        },
        msg_type,
    )
}

#[test]
fn should_parse_simple_comm() {
    let parsed =
        parse_stream(Http2, parse_test_xml(
            r#"
  <proto name="http2" showname="HyperText Transfer Protocol 2" size="341" pos="0">
    <field name="http2.stream" showname="Stream: HEADERS, Stream ID: 1, Length 332, GET /libraries/gbuemRf7.js" size="341" pos="0" show="" value="">
      <field name="http2.length" showname="Length: 332" size="3" pos="0" show="332" value="00014c"/>
      <field name="http2.type" showname="Type: HEADERS (1)" size="1" pos="3" show="1" value="01"/>
      <field name="http2.flags" showname="Flags: 0x25, Priority, End Headers, End Stream" size="1" pos="4" show="0x00000025" value="25">
        <field name="http2.flags.unused_headers" showname="00.0 ..0. = Unused: 0x00" size="1" pos="4" show="0x00000000" value="0" unmaskedvalue="25"/>
        <field name="http2.flags.priority" showname="..1. .... = Priority: True" size="1" pos="4" show="1" value="1" unmaskedvalue="25"/>
        <field name="http2.flags.padded" showname=".... 0... = Padded: False" size="1" pos="4" show="0" value="0" unmaskedvalue="25"/>
        <field name="http2.flags.eh" showname=".... .1.. = End Headers: True" size="1" pos="4" show="1" value="1" unmaskedvalue="25"/>
        <field name="http2.flags.end_stream" showname=".... ...1 = End Stream: True" size="1" pos="4" show="1" value="1" unmaskedvalue="25"/>
      </field>
      <field name="http2.r" showname="0... .... .... .... .... .... .... .... = Reserved: 0x0" size="4" pos="5" show="0x00000000" value="0" unmaskedvalue="00000001"/>
      <field name="http2.streamid" showname=".000 0000 0000 0000 0000 0000 0000 0001 = Stream Identifier: 1" size="4" pos="5" show="1" value="1" unmaskedvalue="00000001"/>
      <field name="http2.pad_length" showname="Pad Length: 0" size="0" pos="9" show="0"/>
      <field name="http2.exclusive" showname="1... .... .... .... .... .... .... .... = Exclusive: True" size="4" pos="9" show="1" value="1" unmaskedvalue="80000000"/>
      <field name="http2.stream_dependency" showname=".000 0000 0000 0000 0000 0000 0000 0000 = Stream Dependency: 0" size="4" pos="9" show="0" value="0" unmaskedvalue="80000000"/>
      <field name="http2.headers.weight" showname="Weight: 182" size="1" pos="13" show="182" value="b6"/>
      <field name="http2.headers.weight_real" showname="Weight real: 183" size="1" pos="13" show="183" value="b6"/>
      <field name="http2.headers" showname="Header Block Fragment: 82418c24952fd3c5740fd16c5c87a787049062834760ec3150c4d1da5a76caeafd114087…" size="327" pos="14" show="82:41:8c:24:95:2f:d3:c5:74:0f:d1:6c:5c:87:a7:87:04:90:62:83:47:60:ec:31:50:c4:d1:da:5a:76:ca:ea:fd:11:40:87:41:48:b1:27:5a:d1:ff:b7:fe:54:d2:74:a9:0f:dd:db:07:54:9f:cf:df:78:3f:97:df:fe:7e:94:fe:6f:4f:61:e9:35:b4:ff:3f:7d:e0:fe:5f:07:f3:f4:a7:f3:88:e7:9a:82:a9:7a:7b:0f:49:7f:9f:be:f0:7f:2f:83:f9:40:8b:41:48:b1:27:5a:d1:ad:49:e3:35:05:02:3f:30:7a:d6:d0:7f:66:a2:81:b0:da:e0:53:fa:fc:08:7e:d4:c2:59:0f:60:fe:d4:ce:6a:ad:f2:a7:97:9c:89:c6:bf:b5:21:ae:ba:0b:c8:b1:e6:32:58:6d:97:57:65:c5:3f:ac:d8:f7:e8:cf:f4:a5:06:ea:55:31:14:9d:4f:fd:a9:7a:7b:0f:49:58:7c:0b:81:76:9a:64:0b:ba:25:37:0e:51:d8:66:1b:65:d5:d9:73:53:03:2a:2f:2a:40:8a:41:48:b4:a5:49:27:59:06:49:7f:87:25:87:42:16:41:92:5f:40:8a:41:48:b4:a5:49:27:5a:93:c8:5f:85:a8:eb:10:f6:23:40:8a:41:48:b4:a5:49:27:5a:42:a1:3f:84:41:2c:35:69:73:91:9d:29:ad:17:18:63:c7:8f:0b:d8:9e:e8:a0:eb:a0:cc:7f:50:8d:9b:d9:ab:fa:52:42:cb:40:d2:5f:a5:23:b3:51:9c:2d:4b:62:bb:f4:5a:96:e1:bb:ef:b4:00:5d:ff:a2:d5:f7:da:00:2e:f7:d4:b6:7d:f6:80:0b:bb" value="82418c24952fd3c5740fd16c5c87a787049062834760ec3150c4d1da5a76caeafd1140874148b1275ad1ffb7fe54d274a90fdddb07549fcfdf783f97dffe7e94fe6f4f61e935b4ff3f7de0fe5f07f3f4a7f388e79a82a97a7b0f497f9fbef07f2f83f9408b4148b1275ad1ad49e33505023f307ad6d07f66a281b0dae053fafc087ed4c2590f60fed4ce6aadf2a7979c89c6bfb521aeba0bc8b1e632586d975765c53facd8f7e8cff4a506ea5531149d4ffda97a7b0f49587c0b81769a640bba25370e51d8661b65d5d97353032a2f2a408a4148b4a549275906497f872587421641925f408a4148b4a549275a93c85f85a8eb10f623408a4148b4a549275a42a13f84412c356973919d29ad171863c78f0bd89ee8a0eba0cc7f508d9bd9abfa5242cb40d25fa523b3519c2d4b62bbf45a96e1bbefb4005dffa2d5f7da002ef7d4b67df6800bbb"/>
      <field name="http2.header.length" showname="Header Length: 585" size="1" pos="0" show="585" value="00"/>
      <field name="http2.header.count" showname="Header Count: 14" size="1" pos="0" show="14" value="00"/>
      <field name="http2.header" showname="Header: :method: GET" size="1" pos="14" show="" value="">
        <field name="http2.header.name.length" showname="Name Length: 7" size="4" pos="14" show="7" value="00000007"/>
        <field name="http2.header.name" showname="Name: :method" size="7" pos="18" show=":method" value="3a6d6574686f64"/>
        <field name="http2.header.value.length" showname="Value Length: 3" size="4" pos="25" show="3" value="00000003"/>
        <field name="http2.header.value" showname="Value: GET" size="3" pos="15" show="GET" value="474554"/>
        <field name="http2.headers.method" showname=":method: GET" size="3" pos="15" show="GET" value="474554"/>
        <field name="http2.header.unescaped" showname="Unescaped: GET" size="3" pos="15" show="GET" value="474554"/>
        <field name="http2.header.repr" showname="Representation: Indexed Header Field" size="1" pos="14" show="Indexed Header Field" value="82"/>
        <field name="http2.header.index" showname="Index: 2" size="1" pos="14" show="2" value="82"/>
      </field>
      <field name="http2.header" showname="Header: :authority: cdn.jwplayer.com" size="14" pos="15" show="" value="">
        <field name="http2.header.name.length" showname="Name Length: 10" size="4" pos="18" show="10" value="0000000a"/>
        <field name="http2.header.name" showname="Name: :authority" size="10" pos="22" show=":authority" value="3a617574686f72697479"/>
        <field name="http2.header.value.length" showname="Value Length: 16" size="4" pos="32" show="16" value="00000010"/>
        <field name="http2.header.value" showname="Value: cdn.jwplayer.com" size="16" pos="36" show="cdn.jwplayer.com" value="63646e2e6a77706c617965722e636f6d"/>
        <field name="http2.headers.authority" showname=":authority: cdn.jwplayer.com" size="16" pos="36" show="cdn.jwplayer.com" value="63646e2e6a77706c617965722e636f6d"/>
        <field name="http2.header.unescaped" showname="Unescaped: cdn.jwplayer.com" size="16" pos="36" show="cdn.jwplayer.com" value="63646e2e6a77706c617965722e636f6d"/>
        <field name="http2.header.repr" showname="Representation: Literal Header Field with Incremental Indexing - Indexed Name" size="1" pos="15" show="Literal Header Field with Incremental Indexing - Indexed Name" value="41"/>
        <field name="http2.header.index" showname="Index: 1" size="1" pos="15" show="1" value="41"/>
      </field>
      <field name="http2.header" showname="Header: :path: /libraries/gbuemRf7.js" size="18" pos="30" show="" value="">
        <field name="http2.header.name.length" showname="Name Length: 5" size="4" pos="72" show="5" value="00000005"/>
        <field name="http2.header.name" showname="Name: :path" size="5" pos="76" show=":path" value="3a70617468"/>
        <field name="http2.header.value.length" showname="Value Length: 22" size="4" pos="81" show="22" value="00000016"/>
        <field name="http2.header.value" showname="Value: /libraries/gbuemRf7.js" size="22" pos="85" show="/libraries/gbuemRf7.js" value="2f6c69627261726965732f676275656d5266372e6a73"/>
        <field name="http2.headers.path" showname=":path: /libraries/gbuemRf7.js" size="22" pos="85" show="/libraries/gbuemRf7.js" value="2f6c69627261726965732f676275656d5266372e6a73"/>
        <field name="http2.header.unescaped" showname="Unescaped: /libraries/gbuemRf7.js" size="22" pos="85" show="/libraries/gbuemRf7.js" value="2f6c69627261726965732f676275656d5266372e6a73"/>
        <field name="http2.header.repr" showname="Representation: Literal Header Field without Indexing - Indexed Name" size="1" pos="30" show="Literal Header Field without Indexing - Indexed Name" value="04"/>
        <field name="http2.header.index" showname="Index: 4" size="1" pos="30" show="4" value="04"/>
      </field>
    </field>
  </proto>
    "#,
        ))
        .unwrap().messages;
    let expected = vec![MessageData::Http(HttpMessageData {
        http_stream_id: 1,
        request: Some(HttpRequestResponseData {
            tcp_stream_no: 4,
            tcp_seq_number: 1963007432,
            timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            first_line: "GET /libraries/gbuemRf7.js".to_string(),
            headers: vec![
                (":method".into(), "GET".into()),
                (":authority".into(), "cdn.jwplayer.com".into()),
                (":path".into(), "/libraries/gbuemRf7.js".into()),
            ],
            body: HttpBody::Missing,
            content_type: None,
            content_encoding: ContentEncoding::Plain,
        }),
        response: None,
    })];
    assert_eq!(expected, parsed);
    // assert!(false);
}
