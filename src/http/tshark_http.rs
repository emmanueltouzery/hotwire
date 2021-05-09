use crate::tshark_communication;
use quick_xml::events::Event;
use std::io::BufReader;
use std::process::ChildStdout;

#[derive(Debug, Copy, Clone)]
pub enum HttpType {
    Request,
    Response,
}

#[derive(Debug)]
pub struct TSharkHttp {
    pub http_type: HttpType,
    pub http_host: Option<String>,
    pub first_line: String,
    pub other_lines: String,
    pub body: Option<String>,
    pub content_type: Option<String>,
}

pub fn parse_http_info(
    xml_reader: &mut quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> TSharkHttp {
    let mut http_type;
    let mut http_host;
    let mut first_line;
    let mut other_lines = vec![];
    let mut body;
    let mut content_type;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"") => {
                            first_line = String::from_utf8(
                                tshark_communication::element_attr_val(e, b"show").to_vec(),
                            )
                            .unwrap();
                        }
                        Some(b"http.content_type") => {
                            content_type = String::from_utf8(
                                tshark_communication::element_attr_val(e, b"show").to_vec(),
                            )
                            .ok();
                        }
                        Some(b"http.host") => {
                            http_host = String::from_utf8(
                                tshark_communication::element_attr_val(e, b"show").to_vec(),
                            )
                            .ok();
                        }
                        Some(b"http.request.line") => {
                            other_lines.push(
                                String::from_utf8(
                                    tshark_communication::element_attr_val(e, b"show").to_vec(),
                                )
                                .unwrap(),
                            );
                        }
                        Some(b"http.file_data") => {
                            body = String::from_utf8(
                                tshark_communication::element_attr_val(e, b"show").to_vec(),
                            )
                            .ok();
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return TSharkHttp {
                        http_type,
                        http_host,
                        first_line,
                        other_lines: other_lines.join(""),
                        body,
                        content_type,
                    };
                }
            }
        }
    }
}
