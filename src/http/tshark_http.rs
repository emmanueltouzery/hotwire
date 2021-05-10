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
    let mut http_type = None;
    let mut http_host = None;
    let mut first_line = None;
    let mut other_lines = vec![];
    let mut body = None;
    let mut content_type = None;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                        .map(|kv| kv.unwrap().value);
                    match name.as_deref() {
                        Some(b"") => {
                            first_line = tshark_communication::element_attr_val_string(e, b"show")
                        }
                        Some(b"http.content_type") => {
                            content_type = tshark_communication::element_attr_val_string(e, b"show")
                        }
                        Some(b"http.host") => {
                            http_host = tshark_communication::element_attr_val_string(e, b"show")
                        }
                        Some(b"http.request.line") => {
                            http_type = Some(HttpType::Request);
                            other_lines.push(
                                tshark_communication::element_attr_val_string(e, b"show").unwrap(),
                            );
                        }
                        Some(b"http.response.line") => {
                            http_type = Some(HttpType::Response);
                            other_lines.push(
                                tshark_communication::element_attr_val_string(e, b"show").unwrap(),
                            );
                        }
                        Some(b"http.file_data") => {
                            body = tshark_communication::element_attr_val_string(e, b"show")
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return TSharkHttp {
                        http_type: http_type.unwrap(),
                        http_host,
                        first_line: first_line.unwrap_or_default(),
                        other_lines: other_lines.join(""),
                        body,
                        content_type,
                    };
                }
            }
            _ => {}
        }
    }
}
