use crate::tshark_communication;
use quick_xml::events::Event;
use std::fmt::Debug;
use std::io::BufReader;
use std::process::ChildStdout;
use std::str;

// if i have data over 2 http2 packets, tshark will often give me
// the first part of the data in the first packet, then
// the RECOMPOSED data in the second packet, repeating data from
// the first packet.
// => when combining packets, the recomposed data should overwrite
// previously collected data
pub enum Http2Data {
    BasicData(Vec<u8>),
    RecomposedData(Vec<u8>),
}

impl Debug for Http2Data {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match &self {
            Http2Data::BasicData(d) => fmt.write_str(&format!("BasicData(length: {})", d.len())),
            Http2Data::RecomposedData(d) => {
                fmt.write_str(&format!("RecomposedData(length: {})", d.len()))
            }
        }
    }
}

impl Http2Data {
    fn is_empty(&self) -> bool {
        match &self {
            Http2Data::BasicData(v) => v.is_empty(),
            Http2Data::RecomposedData(v) => v.is_empty(),
        }
    }
}

#[derive(Debug)]
pub struct TSharkHttp2Message {
    pub headers: Vec<(String, String)>,
    pub data: Option<Http2Data>,
    pub stream_id: u32,
    pub is_end_stream: bool,
}

fn parse_http2_info(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> Vec<TSharkHttp2Message> {
    let mut streams = vec![];
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"http2.stream") => {
                            streams.push(parse_http2_stream(xml_reader, buf));
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return streams;
                }
            }
        }
    }
}

fn parse_http2_stream(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> TSharkHttp2Message {
    let mut headers;
    let mut data;
    let mut stream_id;
    let mut is_end_stream;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"http2.streamid") => {
                            stream_id =
                                str::from_utf8(tshark_communication::element_attr_val(e, b"show"))
                                    .unwrap()
                                    .parse()
                                    .unwrap();
                        }
                        Some(b"http2.header") => {
                            headers = parse_http2_headers(xml_reader, buf);
                        }
                        Some(b"http2.flags.end_stream") => {
                            is_end_stream =
                                str::from_utf8(tshark_communication::element_attr_val(e, b"show"))
                                    .unwrap()
                                    == "1";
                        }
                        Some(b"http2.data.data") => {
                            // TODO diff basic/recomposed data relevant in pdml?
                            data = hex::decode(
                                String::from_utf8(
                                    tshark_communication::element_attr_val(e, b"show").to_vec(),
                                )
                                .unwrap()
                                .replace(':', ""),
                            )
                            .ok()
                            .map(Http2Data::RecomposedData);
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"field" {
                    return TSharkHttp2Message {
                        headers,
                        data,
                        stream_id,
                        is_end_stream,
                    };
                }
            }
        }
    }
}

fn parse_http2_headers(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> Vec<(String, String)> {
    let mut cur_name;
    let mut headers = vec![];
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"http2.header.name") => {
                            cur_name = String::from_utf8(
                                tshark_communication::element_attr_val(e, b"show").to_vec(),
                            )
                            .unwrap();
                        }
                        Some(b"http2.header.value") => {
                            headers.push((
                                cur_name,
                                String::from_utf8(
                                    tshark_communication::element_attr_val(e, b"show").to_vec(),
                                )
                                .unwrap(),
                            ));
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"field" {
                    return headers;
                }
            }
        }
    }
}
