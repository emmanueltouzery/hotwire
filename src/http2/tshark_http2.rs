use crate::tshark_communication;
use quick_xml::events::Event;
use std::fmt::Debug;
use std::io::BufRead;

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

pub fn parse_http2_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
    buf: &mut Vec<u8>,
) -> Vec<TSharkHttp2Message> {
    let mut streams = vec![];
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                        .map(|kv| kv.unwrap().value);
                    match name.as_deref() {
                        Some(b"http2.stream") => {
                            streams.push(parse_http2_stream(xml_reader, buf));
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return streams;
                }
            }
            _ => {}
        }
    }
}

fn parse_http2_stream<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
    buf: &mut Vec<u8>,
) -> TSharkHttp2Message {
    let mut headers = vec![];
    let mut data = None;
    let mut stream_id = 0;
    let mut is_end_stream = false;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                        .map(|kv| kv.unwrap().value);
                    match name.as_deref() {
                        Some(b"http2.streamid") => {
                            stream_id =
                                tshark_communication::element_attr_val_number(e, b"show").unwrap();
                        }
                        Some(b"http2.header") => {
                            headers = parse_http2_headers(xml_reader, buf);
                        }
                        Some(b"http2.flags.end_stream") => {
                            is_end_stream =
                                tshark_communication::element_attr_val_number(e, b"show")
                                    == Some(1);
                        }
                        Some(b"http2.data.data") => {
                            // TODO diff basic/recomposed data relevant in pdml?
                            data = hex::decode(
                                tshark_communication::element_attr_val_string(e, b"show")
                                    .unwrap()
                                    .replace(':', ""),
                            )
                            .ok()
                            .map(Http2Data::RecomposedData);
                        }
                        _ => {}
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
            _ => {}
        }
    }
}

fn parse_http2_headers<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
    buf: &mut Vec<u8>,
) -> Vec<(String, String)> {
    let mut cur_name = None;
    let mut headers = vec![];
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                        .map(|kv| kv.unwrap().value);
                    match name.as_deref() {
                        Some(b"http2.header.name") => {
                            cur_name = tshark_communication::element_attr_val_string(e, b"show");
                        }
                        Some(b"http2.header.value") => {
                            headers.push((
                                cur_name.take().unwrap(),
                                tshark_communication::element_attr_val_string(e, b"show").unwrap(),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"field" {
                    return headers;
                }
            }
            _ => {}
        }
    }
}
