use crate::tshark_communication;
use quick_xml::events::Event;
use std::fmt::Debug;
use std::io::BufRead;

#[derive(Debug)]
pub struct TSharkHttp2Message {
    pub headers: Vec<(String, String)>,
    pub data: Option<Vec<u8>>,
    pub stream_id: u32,
    pub is_end_stream: bool,
}

pub fn parse_http2_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<Vec<TSharkHttp2Message>, String> {
    let mut streams = vec![];
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Start(ref e)) => {
            if e.name() == b"field" {
                let name = tshark_communication::attr_by_name(&mut e.attributes(), b"name")?;
                if name.as_deref() == Some(b"http2.stream")  {
                    let msg = parse_http2_stream(xml_reader)?;
                    if !msg.headers.is_empty() || matches!(&msg.data, Some(v) if !v.is_empty()) {
                        streams.push(msg);
                    }
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return Ok(streams);
            }
        }
    )
}

fn parse_http2_stream<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<TSharkHttp2Message, String> {
    let mut field_depth = 0;
    let mut headers = vec![];
    let mut data = None;
    let mut stream_id = 0;
    let mut is_end_stream = false;
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = tshark_communication::attr_by_name(&mut e.attributes(), b"name")?;
                match name.as_deref() {
                    Some(b"http2.streamid") => {
                        if let Some(s) = tshark_communication::element_attr_val_number(e, b"show")? {
                            stream_id = s;
                        }
                    }
                    Some(b"http2.flags.end_stream") => {
                        is_end_stream =
                            tshark_communication::element_attr_val_number(e, b"show")?
                                == Some(1);
                    }
                    Some(b"http2.data.data") => {
                        if let Some(data_hex) = tshark_communication::element_attr_val_string(e, b"show")? {
                            data = hex::decode(data_hex.replace(':', "")).ok();
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::Start(ref e)) => {
            if e.name() == b"field" {
                field_depth += 1;
                let name = tshark_communication::attr_by_name(&mut e.attributes(), b"name")?;
                if name.as_deref() == Some(b"http2.header") {
                    headers.append(&mut parse_http2_headers(xml_reader)?);
                    field_depth -= 1; // assume the function parsed the </field>
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"field" {
                field_depth -= 1;
                if field_depth < 0 {
                    return Ok(TSharkHttp2Message {
                        headers,
                        data,
                        stream_id,
                        is_end_stream,
                    });
                }
            }
        }
    )
}

fn parse_http2_headers<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<Vec<(String, String)>, String> {
    let mut cur_name = None;
    let mut headers = vec![];
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = tshark_communication::attr_by_name(&mut e.attributes(), b"name")?;
                match name.as_deref() {
                    Some(b"http2.header.name") => {
                        cur_name = tshark_communication::element_attr_val_string(e, b"show")?;
                    }
                    Some(b"http2.header.value") => {
                        if let (Some(h), Some(v)) =
                            (cur_name.take(), tshark_communication::element_attr_val_string(e, b"show")?) {
                            headers.push((h, v));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"field" {
                return Ok(headers);
            }
        }
    )
}
