use crate::tshark_communication;
use quick_xml::events::Event;
use std::io::BufRead;

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
    pub body: Option<Vec<u8>>,
    pub content_type: Option<String>,
}

pub fn parse_http_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<TSharkHttp, String> {
    let mut http_type = None;
    let mut http_host = None;
    let mut first_line = None;
    let mut other_lines = vec![];
    let mut body = None;
    let mut content_type = None;
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Start(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"") && first_line.is_none() {
                    first_line = tshark_communication::element_attr_val_string(e, b"show")
                        .map(|t| t.trim_end_matches("\\r\\n").to_string())
                }
            }
        }
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                match name.as_deref() {
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
                        // binary will be in "value", text in "show"
                        body = hex::decode(
                            tshark_communication::element_attr_val_string(e, b"value")
                                .or_else(|| tshark_communication::element_attr_val_string(e, b"show"))
                                .unwrap()
                                .replace(':', ""),
                        )
                        .ok();
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return Ok(TSharkHttp {
                    http_type: http_type.unwrap(),
                    http_host,
                    first_line: first_line.unwrap_or_default(),
                    other_lines: other_lines.join(""),
                    body,
                    content_type,
                });
            }
        }
    )
}
